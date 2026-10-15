<script lang="ts">
  import { untrack } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { t } from '../i18n'
  import type { SftpEntry, TrackingInfo } from './types'

  let {
    connId,
    cwd,
    liveIds,
    wide = false,
    onwiden,
    tracking,
    onenabletracking,
  }: {
    /** Connessione SSH della scheda attiva (null se non c'è o non è collegata). */
    connId: number | null
    /** Directory corrente del terminale, comunicata dalla shell (OSC 7). */
    cwd: string
    /** Connessioni delle schede ancora aperte: lo stato delle altre viene liberato. */
    liveIds: number[]
    /** La sidebar è allargata (modalità larga). */
    wide?: boolean
    /** Allarga o ripristina la sidebar. */
    onwiden?: () => void
    /** Stato del tracciamento della directory del terminale. */
    tracking: TrackingInfo
    /** Attiva a mano il tracciamento. */
    onenabletracking: () => void
  } = $props()

  interface PanelState {
    path: string
    entries: SftpEntry[]
    loading: boolean
    error: string
    history: string[]
    index: number
    seq: number
  }

  type SortKey = 'name' | 'size' | 'modified'

  let states = $state<Record<number, PanelState>>({})
  let follow = $state(true)
  let showHidden = $state(false)
  let filter = $state('')
  let sortKey = $state<SortKey>('name')
  let sortAsc = $state(true)
  let selected = $state<string | null>(null)
  let pathInput = $state('')

  const cur = $derived(connId !== null ? states[connId] : undefined)

  const rows = $derived.by(() => {
    if (!cur) return []
    const q = filter.trim().toLowerCase()
    const list = cur.entries.filter(
      (e) => (showHidden || !e.name.startsWith('.')) && (!q || e.name.toLowerCase().includes(q)),
    )
    const dir = sortAsc ? 1 : -1
    return list.sort((a, b) => {
      // Le cartelle stanno sempre prima, qualunque sia l'ordinamento.
      if ((a.kind === 'dir') !== (b.kind === 'dir')) return a.kind === 'dir' ? -1 : 1
      let r = 0
      if (sortKey === 'size') r = a.size - b.size
      else if (sortKey === 'modified') r = (a.modified ?? 0) - (b.modified ?? 0)
      if (r === 0) r = a.name.localeCompare(b.name, undefined, { sensitivity: 'base', numeric: true })
      return r * dir
    })
  })

  function ensure(id: number): PanelState {
    if (!states[id]) {
      states[id] = { path: '', entries: [], loading: false, error: '', history: [], index: -1, seq: 0 }
    }
    return states[id]
  }

  async function go(id: number, path: string, push = true) {
    const st = ensure(id)
    const seq = ++st.seq
    st.loading = true
    st.error = ''
    try {
      const entries = await invoke<SftpEntry[]>('sftp_list', { id, path })
      if (seq !== st.seq) return // una navigazione più recente ha preso il posto di questa
      st.entries = entries
      st.path = path
      selected = null
      if (push) {
        st.history = [...st.history.slice(0, st.index + 1), path]
        st.index = st.history.length - 1
      }
    } catch (e) {
      if (seq === st.seq) st.error = String(e)
    } finally {
      if (seq === st.seq) st.loading = false
    }
  }

  async function init(id: number) {
    const st = ensure(id)
    st.loading = true
    try {
      // Se il terminale ha già comunicato la directory corrente si parte da lì, altrimenti dalla home.
      // (`cwd` si legge dopo l'attesa: nel frattempo il terminale può aver cambiato directory.)
      const home = await invoke<string>('sftp_home', { id })
      await go(id, follow && cwd ? cwd : home)
    } catch (e) {
      st.error = String(e)
      st.loading = false
    }
  }

  // Nuova connessione: inizializza la sua vista.
  const known = new Set<number>()
  $effect(() => {
    const id = connId
    if (id !== null && !known.has(id)) {
      known.add(id)
      void init(id)
    }
  })

  // Connessioni chiuse: libera le loro viste (la vista di una scheda in secondo piano resta).
  $effect(() => {
    const live = new Set(liveIds)
    for (const key of Object.keys(states)) {
      const id = Number(key)
      if (!live.has(id)) {
        delete states[id]
        known.delete(id)
      }
    }
  })

  // Segue il terminale: scatta solo quando cambia la directory del terminale, la connessione o
  // l'aggancio. La navigazione manuale nel pannello non deve essere annullata.
  $effect(() => {
    const id = connId
    const target = cwd
    if (id === null || !follow || !target) return
    untrack(() => {
      const st = states[id]
      if (st && st.path !== target) void go(id, target)
    })
  })

  // Il campo del percorso rispecchia la cartella mostrata.
  $effect(() => {
    pathInput = cur?.path ?? ''
  })

  function parentOf(path: string): string {
    const trimmed = path.replace(/\/+$/, '')
    const cut = trimmed.lastIndexOf('/')
    return cut <= 0 ? '/' : trimmed.slice(0, cut)
  }

  function goUp() {
    if (connId !== null && cur && cur.path !== '/') void go(connId, parentOf(cur.path))
  }

  function goHistory(delta: number) {
    if (connId === null || !cur) return
    const i = cur.index + delta
    if (i < 0 || i >= cur.history.length) return
    cur.index = i
    void go(connId, cur.history[i], false)
  }

  function submitPath(e: Event) {
    e.preventDefault()
    const p = pathInput.trim()
    if (connId !== null && p) void go(connId, p)
  }

  function open(e: SftpEntry) {
    if (connId !== null && e.kind === 'dir') void go(connId, e.path)
  }

  function sortBy(key: SortKey) {
    if (sortKey === key) sortAsc = !sortAsc
    else {
      sortKey = key
      sortAsc = true
    }
  }

  function formatSize(n: number): string {
    if (n < 1024) return `${n} B`
    const units = ['KiB', 'MiB', 'GiB', 'TiB']
    let v = n
    let u = -1
    do {
      v /= 1024
      u++
    } while (v >= 1024 && u < units.length - 1)
    return `${v.toFixed(v < 10 ? 1 : 0)} ${units[u]}`
  }

  function formatDate(secs: number | null): string {
    if (secs === null) return ''
    return new Date(secs * 1000).toLocaleString(undefined, {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
    })
  }

  /** Permessi in forma `rwxr-xr-x`, per la descrizione al passaggio del mouse. */
  function modeString(e: SftpEntry): string {
    if (e.mode === null) return ''
    const bits = 'rwxrwxrwx'
    let out = ''
    for (let i = 0; i < 9; i++) out += e.mode & (1 << (8 - i)) ? bits[i] : '-'
    return `${e.kind === 'dir' ? 'd' : e.isSymlink ? 'l' : '-'}${out}`
  }

  const icon = (e: SftpEntry) => (e.kind === 'dir' ? '📁' : e.isSymlink ? '🔗' : '📄')

  function onkey(e: KeyboardEvent, entry: SftpEntry) {
    if (e.key === 'Enter') {
      e.preventDefault()
      open(entry)
    } else if (e.key === 'Backspace') {
      e.preventDefault()
      goUp()
    }
  }
</script>

<section class="sftp" aria-label={t('sftp')}>
  <div class="head">
    <strong>{t('sftp')}</strong>
    <span class="spacer"></span>
    <button type="button" title={t('back')} aria-label={t('back')} disabled={!cur || cur.index <= 0} onclick={() => goHistory(-1)}>←</button>
    <button type="button" title={t('forward')} aria-label={t('forward')} disabled={!cur || cur.index >= cur.history.length - 1} onclick={() => goHistory(1)}>→</button>
    <button type="button" title={t('parentFolder')} aria-label={t('parentFolder')} disabled={!cur || cur.path === '/'} onclick={goUp}>↑</button>
    <button type="button" title={t('refresh')} aria-label={t('refresh')} disabled={!cur || connId === null} onclick={() => connId !== null && cur && go(connId, cur.path, false)}>↻</button>
    <button
      type="button"
      class:on={follow}
      aria-pressed={follow}
      title={follow ? t('followOn') : t('followOff')}
      aria-label={t('followTerminal')}
      onclick={() => (follow = !follow)}>{follow ? '🔒' : '🔓'}</button
    >
    {#if onwiden}
      <button type="button" class:on={wide} aria-pressed={wide} title={wide ? t('narrow') : t('widen')} aria-label={wide ? t('narrow') : t('widen')} onclick={onwiden}>⇔</button>
    {/if}
    <button type="button" class:on={showHidden} aria-pressed={showHidden} title={t('showHidden')} aria-label={t('showHidden')} onclick={() => (showHidden = !showHidden)}>.*</button>
  </div>

  {#if connId === null || !cur}
    <p class="empty">{t('sftpHint')}</p>
  {:else}
    {#if follow && tracking.state !== 'active'}
      <div class="notice" role="status">
        {#if tracking.state === 'pending'}
          {t('trackingPending')}
        {:else if tracking.state === 'unsupported'}
          {t('trackingUnsupported', { shell: (tracking.shell ?? '').split('/').pop() ?? '' })}
        {:else if tracking.state === 'off'}
          {t('trackingOff')}
          <button type="button" onclick={onenabletracking}>{t('injectCwd')}</button>
        {:else}
          {t('trackingError')}{tracking.message ? `: ${tracking.message}` : ''}
          <button type="button" onclick={onenabletracking}>{t('retry')}</button>
        {/if}
      </div>
    {/if}
    <form class="pathbar" onsubmit={submitPath}>
      <input bind:value={pathInput} aria-label={t('path')} spellcheck="false" onkeydown={(e) => e.key === 'Escape' && (pathInput = cur.path)} />
    </form>
    <input class="filter" type="search" bind:value={filter} placeholder={t('filterFiles')} aria-label={t('filterFiles')} />

    {#if cur.error}
      <div class="error" role="alert">
        {cur.error}
        <button type="button" onclick={() => connId !== null && go(connId, cur.path || pathInput, false)}>{t('retry')}</button>
      </div>
    {/if}

    <div class="cols" role="row">
      <button type="button" class="c-name" onclick={() => sortBy('name')}>{t('name')}{sortKey === 'name' ? (sortAsc ? ' ▲' : ' ▼') : ''}</button>
      <button type="button" class="c-size" onclick={() => sortBy('size')}>{t('size')}{sortKey === 'size' ? (sortAsc ? ' ▲' : ' ▼') : ''}</button>
      <button type="button" class="c-date" onclick={() => sortBy('modified')}>{t('modified')}{sortKey === 'modified' ? (sortAsc ? ' ▲' : ' ▼') : ''}</button>
    </div>

    <div class="list" role="listbox" aria-label={t('sftp')} aria-busy={cur.loading}>
      {#each rows as e (e.path)}
        <div
          class="row"
          class:selected={selected === e.path}
          role="option"
          aria-selected={selected === e.path}
          tabindex="0"
          title="{e.name}{modeString(e) ? `\n${modeString(e)}` : ''}{e.owner ? `  ${e.owner}${e.group ? ':' + e.group : ''}` : ''}"
          onclick={() => (selected = e.path)}
          ondblclick={() => open(e)}
          onkeydown={(ev) => onkey(ev, e)}
        >
          <span class="c-name"><span class="icon">{icon(e)}</span><span class="nm">{e.name}</span></span>
          <span class="c-size">{e.kind === 'dir' ? '' : formatSize(e.size)}</span>
          <span class="c-date">{formatDate(e.modified)}</span>
        </div>
      {/each}
      {#if cur.loading}
        <p class="hint">{t('loading')}</p>
      {:else if rows.length === 0 && !cur.error}
        <p class="hint">{cur.entries.length === 0 ? t('emptyFolder') : t('noMatches')}</p>
      {/if}
    </div>
  {/if}
</section>

<style>
  .sftp {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    border-top: 1px solid var(--border);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 8px;
  }
  .spacer {
    flex: 1;
  }
  .head button {
    padding: 2px 7px;
    min-width: 26px;
  }
  .head button.on {
    background: color-mix(in srgb, var(--accent) 30%, transparent);
    border-color: var(--accent);
  }
  .pathbar {
    padding: 0 8px 6px;
  }
  .pathbar input,
  .filter {
    width: 100%;
    box-sizing: border-box;
    font-family: Consolas, monospace;
    font-size: 12px;
    padding: 4px 8px;
  }
  .filter {
    width: calc(100% - 16px);
    margin: 0 8px 6px;
  }
  .cols,
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 64px 118px;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
  }
  .cols {
    font-size: 11px;
    color: var(--fg-dim);
    border-bottom: 1px solid var(--border);
  }
  .cols button {
    border: none;
    background: transparent;
    padding: 2px 0;
    text-align: left;
    color: inherit;
    font-size: inherit;
  }
  .c-size,
  .c-date {
    text-align: right;
    font-size: 11px;
    color: var(--fg-dim);
    white-space: nowrap;
  }
  .cols .c-size,
  .cols .c-date {
    text-align: right;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .row {
    cursor: default;
    user-select: none;
    font-size: 13px;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.selected {
    background: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .c-name {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
  }
  .nm {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty,
  .hint {
    padding: 10px 12px;
    margin: 0;
    color: var(--fg-dim);
    font-size: 12px;
  }
  .notice {
    margin: 0 8px 6px;
    padding: 6px 8px;
    border-radius: 4px;
    background: color-mix(in srgb, #f0ad4e 22%, transparent);
    border: 1px solid #f0ad4e;
    font-size: 12px;
    word-break: break-word;
  }
  .notice button {
    margin-left: 6px;
    padding: 1px 8px;
  }
  .error {
    margin: 0 8px 6px;
    padding: 6px 8px;
    border-radius: 4px;
    background: #5a1d1d;
    color: #ffd6d6;
    font-size: 12px;
    word-break: break-word;
  }
  .error button {
    margin-left: 6px;
    padding: 1px 8px;
  }
</style>
