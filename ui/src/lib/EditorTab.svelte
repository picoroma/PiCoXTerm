<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { Compartment, EditorState, Prec, type Text } from '@codemirror/state'
  import {
    EditorView,
    drawSelection,
    highlightActiveLine,
    highlightActiveLineGutter,
    keymap,
    lineNumbers,
  } from '@codemirror/view'
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
  import { bracketMatching, HighlightStyle, indentOnInput, syntaxHighlighting } from '@codemirror/language'
  import { highlightSelectionMatches, search, searchKeymap } from '@codemirror/search'
  import { tags as tg } from '@lezer/highlight'
  import { languageFor } from './editorLang'
  import { lang, t } from '../i18n'
  import type { SaveOutcome, TextFile } from './types'

  let {
    connId,
    path,
    name,
    active,
    dirty = $bindable(false),
  }: {
    connId: number
    path: string
    name: string
    active: boolean
    /** Ci sono modifiche non salvate (la scheda lo mostra e chiede conferma alla chiusura). */
    dirty?: boolean
  } = $props()

  let host = $state<HTMLDivElement>()
  let view: EditorView | undefined
  const wrapCompartment = new Compartment()

  let file = $state.raw<TextFile | null>(null)
  let loading = $state(true)
  let loadError = $state('')
  let error = $state('')
  let saving = $state(false)
  let justSaved = $state(false)
  let wrap = $state(false)
  let line = $state(1)
  let col = $state(1)
  /** Versione del documento già presente sul server: serve a sapere se ci sono modifiche. */
  let savedDoc: Text | undefined
  /** Esito "conflitto" in attesa di una scelta dell'utente. */
  let conflict = $state<{ missing: boolean } | null>(null)
  let savedTimer: ReturnType<typeof setTimeout> | undefined

  const dirOf = (p: string) => p.slice(0, p.length - name.length)

  const eolLabel = $derived(file?.eol === 'crlf' ? 'CRLF' : 'LF')
  const encLabel = $derived(
    file?.encoding === 'utf8-bom' ? 'UTF-8 BOM' : file?.encoding === 'latin1' ? 'ISO-8859-1' : 'UTF-8',
  )

  // Colori della sintassi: variabili CSS (definite in app.css) per seguire il tema chiaro e scuro.
  const highlight = HighlightStyle.define([
    { tag: [tg.keyword, tg.operatorKeyword, tg.modifier], color: 'var(--syn-keyword)' },
    { tag: [tg.string, tg.special(tg.string), tg.regexp], color: 'var(--syn-string)' },
    { tag: [tg.number, tg.bool, tg.null, tg.atom], color: 'var(--syn-number)' },
    { tag: [tg.comment, tg.lineComment, tg.blockComment], color: 'var(--syn-comment)', fontStyle: 'italic' },
    { tag: [tg.propertyName, tg.attributeName, tg.definition(tg.variableName)], color: 'var(--syn-name)' },
    { tag: [tg.typeName, tg.className, tg.tagName, tg.heading], color: 'var(--syn-type)' },
    { tag: [tg.meta, tg.processingInstruction, tg.link, tg.url], color: 'var(--syn-meta)' },
    { tag: tg.invalid, color: 'var(--danger)' },
  ])

  const theme = EditorView.theme({
    '&': { height: '100%', backgroundColor: 'var(--bg)', color: 'var(--fg)' },
    '&.cm-focused': { outline: 'none' },
    '.cm-scroller': { fontFamily: 'Consolas, "Cascadia Mono", monospace', fontSize: '13px', lineHeight: '1.45' },
    '.cm-gutters': { backgroundColor: 'var(--bg-elev)', color: 'var(--fg-dim)', border: 'none' },
    '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--fg) 7%, transparent)' },
    '.cm-activeLineGutter': { backgroundColor: 'var(--bg-hover)' },
    '.cm-cursor': { borderLeftColor: 'var(--fg)' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
      backgroundColor: 'color-mix(in srgb, var(--accent) 45%, transparent)',
    },
    '.cm-selectionMatch': { backgroundColor: 'color-mix(in srgb, var(--accent) 25%, transparent)' },
    '.cm-searchMatch': { backgroundColor: 'color-mix(in srgb, #e5c07b 40%, transparent)', outline: 'none' },
    '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: 'color-mix(in srgb, #e5a000 65%, transparent)' },
    '.cm-panels': { backgroundColor: 'var(--bg-elev)', color: 'var(--fg)', borderColor: 'var(--border)' },
    '.cm-panels input, .cm-panels button': { fontSize: '12px' },
    '.cm-textfield': { backgroundColor: 'var(--bg)', color: 'var(--fg)', border: '1px solid var(--border)', borderRadius: '3px' },
    '.cm-button': {
      backgroundImage: 'none',
      backgroundColor: 'var(--bg-hover)',
      color: 'var(--fg)',
      border: '1px solid var(--border)',
      borderRadius: '3px',
    },
    '.cm-panel.cm-search [name=close]': { color: 'var(--fg-dim)', backgroundColor: 'transparent', border: 'none' },
    '.cm-panels label': { fontSize: '12px' },
    '.cm-matchingBracket': { backgroundColor: 'color-mix(in srgb, var(--accent) 35%, transparent)', outline: 'none' },
  })

  /** Testi del pannello di ricerca di CodeMirror, in italiano se l'interfaccia lo è. */
  const phrases: Record<string, string> =
    lang === 'it'
      ? {
          Find: 'Cerca',
          Replace: 'Sostituisci',
          next: 'avanti',
          previous: 'indietro',
          all: 'tutti',
          'match case': 'maiuscole',
          regexp: 'regexp',
          'by word': 'parola intera',
          replace: 'sostituisci',
          'replace all': 'sostituisci tutti',
          close: 'chiudi',
          'Go to line': 'Vai alla riga',
          go: 'vai',
        }
      : {}

  function lineCol(state: EditorState) {
    const head = state.selection.main.head
    const l = state.doc.lineAt(head)
    line = l.number
    col = head - l.from + 1
  }

  function buildState(content: string): EditorState {
    return EditorState.create({
      doc: content,
      extensions: [
        EditorState.phrases.of(phrases),
        lineNumbers(),
        highlightActiveLineGutter(),
        history(),
        drawSelection(),
        indentOnInput(),
        bracketMatching(),
        highlightActiveLine(),
        highlightSelectionMatches(),
        search({ top: true }),
        syntaxHighlighting(highlight),
        theme,
        languageFor(name),
        wrapCompartment.of(wrap ? EditorView.lineWrapping : []),
        Prec.highest(
          keymap.of([
            {
              key: 'Mod-s',
              preventDefault: true,
              run: () => {
                void save()
                return true
              },
            },
          ]),
        ),
        keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) {
            dirty = savedDoc ? !u.state.doc.eq(savedDoc) : false
            justSaved = false
          }
          if (u.docChanged || u.selectionSet) lineCol(u.state)
        }),
      ],
    })
  }

  async function load() {
    loading = true
    loadError = ''
    try {
      const f = await invoke<TextFile>('sftp_read_text', { id: connId, path })
      file = f
      const state = buildState(f.content)
      savedDoc = state.doc
      dirty = false
      conflict = null
      error = ''
      if (view) view.setState(state)
      else if (host) view = new EditorView({ state, parent: host })
      lineCol(state)
    } catch (e) {
      loadError = String(e)
    } finally {
      loading = false
    }
  }

  async function save(overwrite = false) {
    if (!view || !file || saving) return
    if (!dirty && !overwrite) return
    saving = true
    error = ''
    const doc = view.state.doc
    try {
      const base = overwrite ? null : { size: file.size, modified: file.modified }
      const r = await invoke<SaveOutcome>('sftp_write_text', {
        id: connId,
        path,
        content: doc.toString(),
        encoding: file.encoding,
        eol: file.eol,
        base,
      })
      if (r.status === 'saved') {
        file = { ...file, size: r.size, modified: r.modified, mixedEol: false }
        savedDoc = doc
        dirty = !view.state.doc.eq(doc)
        conflict = null
        justSaved = true
        clearTimeout(savedTimer)
        savedTimer = setTimeout(() => (justSaved = false), 2500)
      } else {
        conflict = { missing: r.missing }
      }
    } catch (e) {
      error = String(e)
    } finally {
      saving = false
    }
  }

  function toggleWrap() {
    wrap = !wrap
    view?.dispatch({ effects: wrapCompartment.reconfigure(wrap ? EditorView.lineWrapping : []) })
  }

  onMount(() => {
    void load()
  })

  onDestroy(() => {
    clearTimeout(savedTimer)
    view?.destroy()
  })

  // Il pannello nascosto non misura: alla riattivazione si ricalcola il layout e si rimette il focus.
  $effect(() => {
    if (active) {
      untrack(() => {
        view?.requestMeasure()
        view?.focus()
      })
    }
  })
</script>

<div class="editor" class:hidden={!active}>
  <div class="bar">
    <span class="path" title={path}><span class="dir">{dirOf(path)}</span><strong>{name}</strong></span>
    {#if dirty}<span class="dirty" title={t('editorUnsaved')}>●</span>{/if}
    {#if saving}<span class="state">{t('editorSaving')}</span>{:else if justSaved}<span class="state ok">{t('editorSaved')}</span>{/if}
    <span class="spacer"></span>
    <label class="check"><input type="checkbox" checked={wrap} onchange={toggleWrap} /> {t('editorWrap')}</label>
    <button type="button" disabled={loading || saving} onclick={() => void load()} title={t('editorReload')}>{t('editorReload')}</button>
    <button type="button" class="primary" disabled={!dirty || saving || loading} onclick={() => void save()} title={t('editorSave')}>{t('save')}</button>
  </div>

  {#if conflict}
    <div class="banner warn" role="alert">
      <strong>{t('editorConflictTitle')}</strong>
      <span>{conflict.missing ? t('editorMissingBody') : t('editorConflictBody')}</span>
      <span class="spacer"></span>
      <button type="button" class="danger" onclick={() => void save(true)}>{t('editorOverwrite')}</button>
      {#if !conflict.missing}
        <button type="button" onclick={() => void load()}>{t('editorDiscardReload')}</button>
      {/if}
      <button type="button" onclick={() => (conflict = null)}>{t('cancel')}</button>
    </div>
  {/if}
  {#if error}
    <div class="banner err" role="alert">
      <span>{error}</span>
      <span class="spacer"></span>
      <button type="button" onclick={() => (error = '')}>{t('close')}</button>
    </div>
  {/if}
  {#if file?.mixedEol && !conflict}
    <div class="banner info" role="status">{t('editorMixedEol', { eol: eolLabel })}</div>
  {/if}

  {#if loadError}
    <div class="loaderr" role="alert">
      <p><strong>{t('editorLoadFailed')}</strong></p>
      <p>{loadError}</p>
      <button type="button" onclick={() => void load()}>{t('retry')}</button>
    </div>
  {:else if loading && !file}
    <p class="loading">{t('editorOpening')}</p>
  {/if}

  <div class="cm" class:gone={!!loadError || (loading && !file)} bind:this={host}></div>

  <div class="status">
    <span title={t('editorMemoryNote')}>{encLabel}</span>
    <span>{eolLabel}</span>
    <span>{t('editorLine', { l: line, c: col })}</span>
    <span class="spacer"></span>
    {#if file}<span>{file.size.toLocaleString()} B</span>{/if}
  </div>
</div>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .editor.hidden {
    display: none;
  }
  .bar,
  .status,
  .banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 10px;
    background: var(--bg-elev);
    font-size: 12px;
  }
  .status {
    color: var(--fg-dim);
    border-top: 1px solid var(--border);
  }
  .path {
    font-family: Consolas, monospace;
    display: flex;
    min-width: 0;
    white-space: nowrap;
  }
  .path .dir {
    overflow: hidden;
    text-overflow: ellipsis; /* se il percorso non entra si accorcia la cartella, non il nome del file */
    color: var(--fg-dim);
  }
  .spacer {
    flex: 1;
  }
  .dirty {
    color: #e5a000;
  }
  .state {
    color: var(--fg-dim);
  }
  .state.ok {
    color: #5cb85c;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
  }
  .banner.warn {
    background: #5a3d1d;
    color: #ffe4c4;
  }
  .banner.err {
    background: color-mix(in srgb, var(--danger) 35%, var(--bg-elev));
  }
  .banner.info {
    color: var(--fg-dim);
  }
  .cm {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .cm.gone {
    display: none;
  }
  .loading,
  .loaderr {
    padding: 24px;
    color: var(--fg-dim);
  }
  .loaderr {
    color: var(--fg);
  }
</style>
