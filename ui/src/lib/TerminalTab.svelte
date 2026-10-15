<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'
  import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager'
  import { Terminal } from '@xterm/xterm'
  import { FitAddon } from '@xterm/addon-fit'
  import { SearchAddon } from '@xterm/addon-search'
  import '@xterm/xterm/css/xterm.css'
  import Modal from './Modal.svelte'
  import { t } from '../i18n'
  import { settings } from './settings.svelte'
  import { terminalThemes } from './themes'
  import type { SessionUi, Target } from './types'

  type TermEvent =
    | { type: 'hostKey'; reqId: number; host: string; port: number; keyType: string; fingerprint: string }
    | { type: 'closed'; exit: number | null }
    | { type: 'cwd'; path: string }

  let {
    target,
    active,
    ui = null,
    savePassword,
  }: {
    target: Target
    active: boolean
    /** Aspetto della sessione salvata; i campi null ereditano dalle impostazioni. */
    ui?: SessionUi | null
    /** Salva la password nella sessione (solo per sessioni salvate). */
    savePassword?: (password: string) => Promise<void>
  } = $props()

  const PASSWORD_REQUIRED = 'password-required'
  const PASSPHRASE_REQUIRED = 'passphrase-required'
  let askKind = $state<'password' | 'passphrase'>('password')

  let status = $state<'connecting' | 'connected' | 'closed'>('connecting')
  let error = $state('')
  let cwd = $state('')
  let injectShell = $state('bash')
  let hostKeyPrompt = $state<Extract<TermEvent, { type: 'hostKey' }> | null>(null)
  let pendingPaste = $state<string | null>(null)
  let askPassword = $state(false)
  let passwordInput = $state('')
  let savePasswordChecked = $state(false)

  let termEl: HTMLDivElement
  let searchInput = $state<HTMLInputElement>()
  let term: Terminal
  let fit: FitAddon
  let searchAddon: SearchAddon
  /** Zoom locale della scheda (Ctrl+rotella), in punti rispetto al font delle impostazioni. */
  let zoom = $state(0)
  let searching = $state(false)
  let query = $state('')
  let caseSensitive = $state(false)
  let noMatch = $state(false)
  let connId: number | null = null
  const encoder = new TextEncoder()

  /** Luminosità percepita di un colore #rrggbb, da 0 a 1. */
  function luminance(hex: string): number {
    const n = parseInt(hex.slice(1), 16)
    return (0.299 * ((n >> 16) & 255) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255
  }

  // Aspetto effettivo: la sessione sovrascrive le impostazioni globali.
  const look = $derived.by(() => {
    const base = terminalThemes[ui?.theme ?? settings.theme]
    const bg = ui?.background ?? null
    // Con uno sfondo personalizzato si sceglie il testo (chiaro o scuro) in base alla sua luminosità.
    const fg = bg ? (luminance(bg) > 0.5 ? '#1e1e1e' : '#e6e6e6') : undefined
    return {
      theme: bg ? { ...base, background: bg, foreground: fg, cursor: fg } : base,
      fontFamily: ui?.fontFamily ?? settings.fontFamily,
      fontSize: ui?.fontSize ?? settings.fontSize,
    }
  })

  async function copySelection() {
    const sel = term?.getSelection()
    if (sel) await writeText(sel)
  }

  async function pasteFromClipboard() {
    const text = await readText().catch(() => '')
    if (!text) return
    if (settings.pasteMultilineWarning && text.trim().includes('\n')) pendingPaste = text
    else term.paste(text)
  }

  function confirmPaste(ok: boolean) {
    const text = pendingPaste
    pendingPaste = null
    if (ok && text) term.paste(text)
    term.focus()
  }

  function runSearch(forward: boolean, incremental = false) {
    if (!query) {
      searchAddon.clearDecorations()
      noMatch = false
      return
    }
    const opts = { caseSensitive, incremental }
    noMatch = !(forward ? searchAddon.findNext(query, opts) : searchAddon.findPrevious(query, opts))
  }

  async function openSearch() {
    searching = true
    await tick()
    searchInput?.focus()
    searchInput?.select()
  }

  function closeSearch() {
    searching = false
    noMatch = false
    searchAddon.clearDecorations()
    term.focus()
  }

  function setZoom(next: number) {
    zoom = Math.max(-6, Math.min(12, next))
  }

  async function connect(password?: string) {
    error = ''
    status = 'connecting'
    cwd = ''
    const onData = new Channel<ArrayBuffer | number[]>()
    onData.onmessage = (buf) => term.write(new Uint8Array(buf))
    const onEvent = new Channel<TermEvent>()
    onEvent.onmessage = (ev) => {
      if (ev.type === 'hostKey') hostKeyPrompt = ev
      else if (ev.type === 'cwd') cwd = ev.path
      else if (ev.type === 'closed') {
        term.write(`\r\n\x1b[33m[${t('closed')}]\x1b[0m\r\n`)
        status = 'closed'
        connId = null
      }
    }
    try {
      connId = await invoke<number>('ssh_connect', {
        request: {
          sessionId: target.sessionId ?? null,
          host: target.host ?? null,
          port: target.port ?? null,
          username: target.username ?? null,
          password: password ?? null,
        },
        cols: term.cols,
        rows: term.rows,
        onData,
        onEvent,
      })
    } catch (e) {
      status = 'closed'
      if (String(e) === PASSWORD_REQUIRED || String(e) === PASSPHRASE_REQUIRED) {
        askKind = String(e) === PASSPHRASE_REQUIRED ? 'passphrase' : 'password'
        passwordInput = ''
        savePasswordChecked = false
        askPassword = true
      } else {
        error = String(e)
      }
      return
    }
    status = 'connected'
    if (password && savePasswordChecked && savePassword) {
      await savePassword(password).catch((e) => (error = String(e)))
    }
    term.focus()
  }

  async function submitPassword(e: Event) {
    e.preventDefault()
    const pw = passwordInput
    passwordInput = ''
    askPassword = false
    await connect(pw)
  }

  async function replyHostKey(accept: boolean) {
    if (!hostKeyPrompt) return
    const reqId = hostKeyPrompt.reqId
    hostKeyPrompt = null
    await invoke('hostkey_reply', { reqId, accept })
  }

  async function injectCwd() {
    if (connId !== null) await invoke('ssh_inject_cwd', { id: connId, shell: injectShell })
    term.focus()
  }

  async function disconnect() {
    if (connId !== null) await invoke('ssh_close', { id: connId })
  }

  onMount(() => {
    term = new Terminal({
      fontFamily: look.fontFamily,
      fontSize: look.fontSize,
      scrollback: settings.scrollback,
      theme: look.theme,
      cursorBlink: true,
    })
    fit = new FitAddon()
    searchAddon = new SearchAddon()
    term.loadAddon(fit)
    term.loadAddon(searchAddon)
    term.open(termEl)
    fit.fit()
    // Copia su selezione; Ctrl+Shift+C / Ctrl+Shift+V come alternativa da tastiera
    term.onSelectionChange(() => {
      if (settings.copyOnSelect) void copySelection()
    })
    term.attachCustomKeyEventHandler((ev) => {
      if (ev.type === 'keydown' && ev.ctrlKey && !ev.shiftKey && ev.code === 'Digit0') {
        setZoom(0)
        return false
      }
      if (ev.type === 'keydown' && ev.ctrlKey && ev.shiftKey) {
        if (ev.code === 'KeyF') {
          void openSearch()
          return false
        }
        if (ev.code === 'KeyC') {
          void copySelection()
          return false
        }
        if (ev.code === 'KeyV') {
          void pasteFromClipboard()
          return false
        }
      }
      return true
    })
    term.onData((s) => {
      if (connId !== null) void invoke('ssh_write', { id: connId, data: Array.from(encoder.encode(s)) })
    })
    const obs = new ResizeObserver(() => {
      fit.fit()
      if (connId !== null) void invoke('ssh_resize', { id: connId, cols: term.cols, rows: term.rows })
    })
    obs.observe(termEl)
    // Ctrl+rotella: zoom del font (listener non passivo, per poter bloccare lo scroll)
    const onWheel = (ev: WheelEvent) => {
      if (!ev.ctrlKey) return
      ev.preventDefault()
      setZoom(zoom + (ev.deltaY < 0 ? 1 : -1))
    }
    termEl.addEventListener('wheel', onWheel, { passive: false })
    void connect()

    return () => {
      termEl.removeEventListener('wheel', onWheel)
      obs.disconnect()
      if (connId !== null) void invoke('ssh_close', { id: connId })
      term.dispose()
    }
  })

  $effect(() => {
    if (active) void tick().then(() => term?.focus())
  })

  // Applica le impostazioni (e lo zoom locale) al terminale già aperto.
  $effect(() => {
    if (!term) return
    term.options.fontFamily = look.fontFamily
    term.options.fontSize = Math.max(6, look.fontSize + zoom)
    term.options.scrollback = settings.scrollback
    term.options.theme = look.theme
    fit.fit()
    if (connId !== null) void invoke('ssh_resize', { id: connId, cols: term.cols, rows: term.rows })
  })
</script>

<div class="tab" class:hidden={!active}>
  <div class="tools">
    {#if status === 'connected'}
      <select bind:value={injectShell} aria-label="shell">
        <option>bash</option>
        <option>tcsh</option>
      </select>
      <button type="button" onclick={injectCwd}>{t('injectCwd')}</button>
      <button type="button" onclick={disconnect}>{t('disconnect')}</button>
    {:else if status === 'connecting'}
      <span class="dim">{t('connecting')}</span>
    {:else}
      <button type="button" class="primary" onclick={() => connect()}>{t('reconnect')}</button>
    {/if}
  </div>
  {#if error}<div class="error">{error}</div>{/if}

  {#if searching}
    <div class="search" role="search">
      <input
        bind:this={searchInput}
        bind:value={query}
        class:nomatch={noMatch}
        placeholder={t('search')}
        aria-label={t('search')}
        oninput={() => runSearch(true, true)}
        onkeydown={(e) => {
          if (e.key === 'Enter') runSearch(!e.shiftKey)
          else if (e.key === 'Escape') closeSearch()
        }}
      />
      <button type="button" title={t('searchPrev')} aria-label={t('searchPrev')} onclick={() => runSearch(false)}>↑</button>
      <button type="button" title={t('searchNext')} aria-label={t('searchNext')} onclick={() => runSearch(true)}>↓</button>
      <label class="case"><input type="checkbox" bind:checked={caseSensitive} onchange={() => runSearch(true, true)} /> Aa</label>
      {#if noMatch}<span class="dim">{t('noMatch')}</span>{/if}
      <button type="button" aria-label={t('close')} onclick={closeSearch}>✕</button>
    </div>
  {/if}
  <div
    class="term"
    role="presentation"
    style:background={look.theme.background}
    bind:this={termEl}
    oncontextmenu={(e) => {
      if (!settings.rightClickPaste) return
      e.preventDefault()
      void pasteFromClipboard()
    }}
  ></div>
  <div class="status">{t('cwd')}: {cwd || '—'}</div>
</div>

{#if askPassword}
  <Modal
    title={askKind === 'passphrase' ? t('passphrasePromptTitle') : t('passwordPromptTitle')}
    onclose={() => (askPassword = false)}
  >
    <form id="pw-form" onsubmit={submitPassword}>
      <input
        type="password"
        bind:value={passwordInput}
        autocomplete="off"
        aria-label={askKind === 'passphrase' ? t('passphrase') : t('password')}
      />
      {#if target.sessionId && savePassword}
        <label class="check"><input type="checkbox" bind:checked={savePasswordChecked} /> {t('savePasswordToo')}</label>
      {/if}
    </form>
    {#snippet actions()}
      <button type="button" onclick={() => (askPassword = false)}>{t('cancel')}</button>
      <button type="submit" form="pw-form" class="primary">{t('connect')}</button>
    {/snippet}
  </Modal>
{/if}

{#if pendingPaste !== null}
  <Modal title={t('pasteTitle')} onclose={() => confirmPaste(false)}>
    <p>{t('pasteBody', { n: pendingPaste.split('\n').length })}</p>
    <pre class="preview">{pendingPaste.slice(0, 400)}</pre>
    {#snippet actions()}
      <button onclick={() => confirmPaste(false)}>{t('cancel')}</button>
      <button class="primary" onclick={() => confirmPaste(true)}>{t('paste')}</button>
    {/snippet}
  </Modal>
{/if}

{#if hostKeyPrompt}
  <Modal title={t('hostKeyTitle')} onclose={() => replyHostKey(false)}>
    <p>{t('hostKeyBody')}</p>
    <dl>
      <dt>{t('host')}</dt>
      <dd>{hostKeyPrompt.host}:{hostKeyPrompt.port}</dd>
      <dt>{t('keyType')}</dt>
      <dd>{hostKeyPrompt.keyType}</dd>
      <dt>{t('fingerprint')}</dt>
      <dd class="mono">{hostKeyPrompt.fingerprint}</dd>
    </dl>
    {#snippet actions()}
      <button onclick={() => replyHostKey(false)}>{t('cancel')}</button>
      <button class="primary" onclick={() => replyHostKey(true)}>{t('accept')}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .tab {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .tab.hidden {
    display: none;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    background: var(--bg-elev);
  }
  .dim {
    color: var(--fg-dim);
  }
  .error {
    padding: 8px;
    background: #5a1d1d;
    color: #ffd6d6;
    white-space: pre-wrap;
  }
  .term {
    flex: 1;
    min-height: 0;
    padding: 4px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    background: var(--bg-elev);
  }
  .search input:not([type='checkbox']) {
    width: 260px;
  }
  .search input.nomatch {
    border-color: var(--danger);
  }
  .case {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 13px;
  }
  .status {
    padding: 4px 8px;
    background: var(--bg-elev);
    font-family: Consolas, monospace;
    font-size: 13px;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  dt {
    font-weight: 600;
    margin-top: 8px;
  }
  dd {
    margin: 0;
  }
  .mono {
    font-family: Consolas, monospace;
    word-break: break-all;
  }
  .preview {
    max-height: 160px;
    overflow: auto;
    background: var(--bg);
    padding: 8px;
    font-size: 12px;
  }
</style>
