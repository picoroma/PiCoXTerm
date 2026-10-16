<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { getCurrentWebview } from '@tauri-apps/api/webview'
  import { open as openDialog } from '@tauri-apps/plugin-dialog'
  import ConflictDialog from './ConflictDialog.svelte'
  import { writeText } from '@tauri-apps/plugin-clipboard-manager'
  import DeleteDialog from './DeleteDialog.svelte'
  import Icon from './Icon.svelte'
  import NameDialog from './NameDialog.svelte'
  import PropertiesDialog from './PropertiesDialog.svelte'
  import { discardPlan, startTransfer, type ConflictPolicy, type PlanSummary } from './transfers.svelte'
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
  let selection = $state(new Set<string>())
  let anchor = $state<string | null>(null)
  let panelEl = $state<HTMLElement>()
  let dropActive = $state(false)
  let actionError = $state('')
  /** Piano in attesa di una scelta sui conflitti. */
  let pending = $state<{ plan: PlanSummary; title: string; onSettled: () => void } | null>(null)
  let menu = $state<{ x: number; y: number; entry: SftpEntry | null } | null>(null)

  type Dialog =
    | { type: 'delete'; entries: SftpEntry[]; busy: boolean; failures: { path: string; message: string }[] }
    | { type: 'rename'; entry: SftpEntry; error: string }
    | { type: 'mkdir'; error: string }
    | { type: 'props'; entry: SftpEntry }
  let dialog = $state<Dialog | null>(null)
  let pathInput = $state('')

  const cur = $derived(connId !== null ? states[connId] : undefined)

  /** Elementi selezionati, per tipo: i file e le cartelle si scaricano con due comandi distinti. */
  const selectedEntries = $derived(cur ? cur.entries.filter((e) => selection.has(e.path)) : [])
  const hasFiles = $derived(selectedEntries.some((e) => e.kind === 'file'))
  const hasDirs = $derived(selectedEntries.some((e) => e.kind === 'dir'))
  /** L'unico elemento selezionato (Rinomina e Proprietà valgono solo per uno alla volta). */
  const single = $derived(selectedEntries.length === 1 ? selectedEntries[0] : null)

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
      selection = new Set()
      anchor = null
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

  /** Selezione come in Esplora risorse: clic, Ctrl+clic per aggiungere/togliere, Maiusc+clic per un intervallo. */
  function selectRow(e: SftpEntry, ev: MouseEvent) {
    if (ev.shiftKey && anchor) {
      const a = rows.findIndex((r) => r.path === anchor)
      const b = rows.findIndex((r) => r.path === e.path)
      if (a >= 0 && b >= 0) {
        const [from, to] = a < b ? [a, b] : [b, a]
        selection = new Set(rows.slice(from, to + 1).map((r) => r.path))
        return
      }
    }
    const next = new Set(ev.ctrlKey || ev.metaKey ? selection : [])
    if ((ev.ctrlKey || ev.metaKey) && next.has(e.path)) next.delete(e.path)
    else next.add(e.path)
    selection = next
    anchor = e.path
  }

  // ---- Trasferimenti ----

  function fmtBytes(n: number): string {
    return formatSize(n)
  }

  /** Mostra la scelta sui conflitti se serve, altrimenti avvia subito. */
  function confirmAndStart(plan: PlanSummary, title: string, onSettled: () => void) {
    if (plan.fileCount === 0 && plan.dirCount === 0) {
      actionError = t('nothingToTransfer')
      void discardPlan(plan)
      return
    }
    if (plan.conflicts > 0) {
      pending = { plan, title, onSettled }
    } else {
      void startTransfer(plan, 'overwrite', title, onSettled)
    }
  }

  function startPending(policy: ConflictPolicy) {
    if (!pending) return
    const p = pending
    pending = null
    void startTransfer(p.plan, policy, p.title, p.onSettled)
  }

  function cancelPending() {
    if (pending) void discardPlan(pending.plan)
    pending = null
  }

  /** Carica file o cartelle locali nella cartella mostrata. */
  async function upload(paths: string[]) {
    if (connId === null || !cur || paths.length === 0) return
    const id = connId
    const dir = cur.path
    actionError = ''
    try {
      const plan = await invoke<PlanSummary>('transfer_plan_upload', { id, localPaths: paths, remoteDir: dir })
      const title = t('uploadTitle', { n: plan.fileCount, size: fmtBytes(plan.totalBytes), dir })
      confirmAndStart(plan, title, () => {
        // Se si sta ancora guardando quella cartella, la si aggiorna.
        if (states[id]?.path === dir) void go(id, dir, false)
      })
    } catch (e) {
      actionError = String(e)
    }
  }

  async function pickUpload(folders: boolean) {
    actionError = ''
    try {
      const picked = await openDialog({ multiple: true, directory: folders, title: folders ? t('uploadFolder') : t('uploadFiles') })
      const paths = Array.isArray(picked) ? picked : typeof picked === 'string' ? [picked] : []
      await upload(paths)
    } catch (e) {
      actionError = String(e)
    }
  }

  /** Scarica in una cartella locale scelta dall'utente i file o le cartelle selezionati. */
  async function downloadSelection(kind: 'file' | 'dir') {
    if (connId === null || !cur) return
    const paths = selectedEntries.filter((e) => e.kind === kind).map((e) => e.path)
    if (paths.length === 0) return
    const id = connId
    actionError = ''
    try {
      const dest = await openDialog({ directory: true, multiple: false, title: t('downloadTo') })
      if (typeof dest !== 'string') return
      const plan = await invoke<PlanSummary>('transfer_plan_download', { id, remotePaths: paths, localDir: dest })
      const title = t('downloadTitle', { n: plan.fileCount, size: fmtBytes(plan.totalBytes), dir: dest })
      confirmAndStart(plan, title, () => {})
    } catch (e) {
      actionError = String(e)
    }
  }

  // ---- Modifica: elimina, rinomina, nuova cartella, proprietà ----

  function refreshCurrent() {
    if (connId !== null && cur) void go(connId, cur.path, false)
  }

  function askDelete() {
    if (selectedEntries.length > 0) dialog = { type: 'delete', entries: [...selectedEntries], busy: false, failures: [] }
  }

  async function confirmDelete() {
    if (dialog?.type !== 'delete' || connId === null) return
    const d = dialog
    d.busy = true
    try {
      const r = await invoke<{ done: number; failed: { path: string; message: string }[] }>('sftp_delete', {
        id: connId,
        paths: d.entries.map((e) => e.path),
      })
      refreshCurrent()
      if (r.failed.length > 0) {
        d.failures = r.failed
        d.busy = false
      } else {
        dialog = null
      }
    } catch (e) {
      actionError = String(e)
      dialog = null
    }
  }

  function askRename() {
    if (single) dialog = { type: 'rename', entry: single, error: '' }
  }

  async function confirmRename(name: string) {
    if (dialog?.type !== 'rename' || connId === null) return
    const d = dialog
    try {
      await invoke('sftp_rename', { id: connId, path: d.entry.path, newName: name })
      dialog = null
      refreshCurrent()
    } catch (e) {
      d.error = String(e)
    }
  }

  function askMkdir() {
    if (cur) dialog = { type: 'mkdir', error: '' }
  }

  async function confirmMkdir(name: string) {
    if (dialog?.type !== 'mkdir' || connId === null || !cur) return
    const d = dialog
    try {
      await invoke('sftp_mkdir', { id: connId, parent: cur.path, name })
      dialog = null
      refreshCurrent()
    } catch (e) {
      d.error = String(e)
    }
  }

  function askProps() {
    if (single) dialog = { type: 'props', entry: single }
  }

  function copyPaths() {
    void writeText(selectedEntries.map((e) => e.path).join('\n'))
  }

  // ---- Menu col tasto destro ----

  function rowMenu(ev: MouseEvent, e: SftpEntry) {
    ev.preventDefault()
    ev.stopPropagation()
    if (!selection.has(e.path)) {
      selection = new Set([e.path])
      anchor = e.path
    }
    menu = { x: ev.clientX, y: ev.clientY, entry: e }
  }

  function emptyMenu(ev: MouseEvent) {
    ev.preventDefault()
    menu = { x: ev.clientX, y: ev.clientY, entry: null }
  }

  function pick(action: () => void) {
    action()
    menu = null
  }

  // ---- Trascinamento da Esplora risorse (evento nativo di Tauri) ----

  function insidePanel(pos: { x: number; y: number }): boolean {
    if (!panelEl) return false
    const r = panelEl.getBoundingClientRect()
    const x = pos.x / window.devicePixelRatio
    const y = pos.y / window.devicePixelRatio
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
  }

  onMount(() => {
    let unlisten: (() => void) | undefined
    let disposed = false
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload
        if (p.type === 'enter' || p.type === 'over') {
          dropActive = connId !== null && !!cur && insidePanel(p.position)
        } else if (p.type === 'drop') {
          const inside = connId !== null && !!cur && insidePanel(p.position)
          dropActive = false
          if (inside) void upload(p.paths)
        } else {
          dropActive = false
        }
      })
      .then((u) => {
        if (disposed) u()
        else unlisten = u
      })
      .catch(() => {
        /* fuori da Tauri (anteprima nel browser) il trascinamento nativo non esiste */
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  })

  function onkey(e: KeyboardEvent, entry: SftpEntry) {
    if (e.key === 'Delete' || e.key === 'F2' || (e.key === 'Enter' && e.altKey)) {
      // I comandi agiscono sulla selezione; se la riga attiva non ne fa parte, si seleziona prima.
      if (!selection.has(entry.path)) {
        selection = new Set([entry.path])
        anchor = entry.path
      }
      e.preventDefault()
      if (e.key === 'Delete') askDelete()
      else if (e.key === 'F2') askRename()
      else askProps()
      return
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
      e.preventDefault()
      selection = new Set(rows.map((r) => r.path))
      return
    }
    if (e.key === 'Enter') {
      e.preventDefault()
      open(entry)
    } else if (e.key === 'Backspace') {
      e.preventDefault()
      goUp()
    }
  }
</script>

<svelte:window onclick={() => (menu = null)} onkeydown={(e) => e.key === 'Escape' && (menu = null)} />

<section class="sftp" class:dropping={dropActive} bind:this={panelEl} aria-label={t('sftp')}>
  <div class="head">
    <span class="group" role="group" aria-label={t('navigation')}>
      <button type="button" title={t('back')} aria-label={t('back')} disabled={!cur || cur.index <= 0} onclick={() => goHistory(-1)}><Icon name="arrow-left" /></button>
      <button type="button" title={t('forward')} aria-label={t('forward')} disabled={!cur || cur.index >= cur.history.length - 1} onclick={() => goHistory(1)}><Icon name="arrow-right" /></button>
      <button type="button" title={t('parentFolder')} aria-label={t('parentFolder')} disabled={!cur || cur.path === '/'} onclick={goUp}><Icon name="parent" /></button>
      <button type="button" title={t('refresh')} aria-label={t('refresh')} disabled={!cur || connId === null} onclick={() => connId !== null && cur && go(connId, cur.path, false)}><Icon name="refresh" /></button>
    </span>
    <span class="group" role="group" aria-label={t('transfers')}>
      <button type="button" title={t('uploadFiles')} aria-label={t('uploadFiles')} disabled={!cur} onclick={() => pickUpload(false)}><Icon name="upload" /></button>
      <button type="button" title={t('uploadFolder')} aria-label={t('uploadFolder')} disabled={!cur} onclick={() => pickUpload(true)}><Icon name="folder-up" /></button>
      <button type="button" title={t('downloadFiles')} aria-label={t('downloadFiles')} disabled={!cur || !hasFiles} onclick={() => downloadSelection('file')}><Icon name="download" /></button>
      <button type="button" title={t('downloadFolders')} aria-label={t('downloadFolders')} disabled={!cur || !hasDirs} onclick={() => downloadSelection('dir')}><Icon name="folder-down" /></button>
    </span>
    <span class="spacer"></span>
    <span class="group" role="group" aria-label={t('viewOptions')}>
      <button
        type="button"
        class:on={follow}
        aria-pressed={follow}
        title={follow ? t('followOn') : t('followOff')}
        aria-label={t('followTerminal')}
        onclick={() => (follow = !follow)}><Icon name={follow ? 'lock' : 'unlock'} /></button
      >
      <button type="button" class:on={showHidden} aria-pressed={showHidden} title={t('showHidden')} aria-label={t('showHidden')} onclick={() => (showHidden = !showHidden)}><Icon name={showHidden ? 'eye' : 'eye-off'} /></button>
      {#if onwiden}
        <button type="button" class:on={wide} aria-pressed={wide} title={wide ? t('narrow') : t('widen')} aria-label={wide ? t('narrow') : t('widen')} onclick={onwiden}><Icon name="arrows-h" /></button>
      {/if}
    </span>
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

    {#if actionError}
      <div class="error" role="alert">{actionError}</div>
    {/if}

    <div class="list" role="listbox" tabindex="-1" aria-multiselectable="true" aria-label={t('sftp')} aria-busy={cur.loading} oncontextmenu={emptyMenu}>
      {#each rows as e (e.path)}
        <div
          class="row"
          class:selected={selection.has(e.path)}
          role="option"
          aria-selected={selection.has(e.path)}
          tabindex="0"
          title="{e.name}{modeString(e) ? `\n${modeString(e)}` : ''}{e.owner ? `  ${e.owner}${e.group ? ':' + e.group : ''}` : ''}"
          onclick={(ev) => selectRow(e, ev)}
          oncontextmenu={(ev) => rowMenu(ev, e)}
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
  {#if dropActive && cur}
    <div class="drop-hint">{t('dropToUpload', { dir: cur.path })}</div>
  {/if}
</section>

{#if pending}
  <ConflictDialog plan={pending.plan} onstart={startPending} oncancel={cancelPending} />
{/if}

{#if menu}
  <div class="menu" role="menu" style:left="{menu.x}px" style:top="{menu.y}px">
    {#if hasFiles}
      <button role="menuitem" onclick={() => pick(() => downloadSelection('file'))}>{t('downloadFiles')}</button>
    {/if}
    {#if hasDirs}
      <button role="menuitem" onclick={() => pick(() => downloadSelection('dir'))}>{t('downloadFolders')}</button>
    {/if}
    {#if menu.entry}<hr />{/if}
    <button role="menuitem" onclick={() => pick(() => pickUpload(false))}>{t('uploadFiles')}</button>
    <button role="menuitem" onclick={() => pick(() => pickUpload(true))}>{t('uploadFolder')}</button>
    <hr />
    <button role="menuitem" onclick={() => pick(askMkdir)}>{t('newFolder')}</button>
    {#if menu.entry}
      <button role="menuitem" disabled={!single} onclick={() => pick(askRename)}>{t('rename')} <kbd>F2</kbd></button>
      <button role="menuitem" class="danger-item" onclick={() => pick(askDelete)}>{t('delete')} <kbd>Canc</kbd></button>
      <hr />
      <button role="menuitem" onclick={() => pick(copyPaths)}>{t('copyPath')}</button>
      <button role="menuitem" disabled={!single} onclick={() => pick(askProps)}>{t('properties')} <kbd>Alt+⏎</kbd></button>
    {/if}
    <hr />
    <button role="menuitem" onclick={() => pick(refreshCurrent)}>{t('refresh')}</button>
  </div>
{/if}

{#if dialog?.type === 'delete'}
  <DeleteDialog
    entries={dialog.entries}
    busy={dialog.busy}
    failures={dialog.failures}
    onconfirm={confirmDelete}
    oncancel={() => (dialog = null)}
  />
{:else if dialog?.type === 'rename'}
  <NameDialog
    title={t('rename')}
    label={t('newName')}
    initial={dialog.entry.name}
    confirmLabel={t('rename')}
    error={dialog.error}
    onconfirm={confirmRename}
    oncancel={() => (dialog = null)}
  />
{:else if dialog?.type === 'mkdir'}
  <NameDialog
    title={t('newFolder')}
    label={t('folderName')}
    confirmLabel={t('create')}
    error={dialog.error}
    onconfirm={confirmMkdir}
    oncancel={() => (dialog = null)}
  />
{:else if dialog?.type === 'props' && connId !== null}
  <PropertiesDialog entry={dialog.entry} {connId} onclose={() => (dialog = null)} onchanged={refreshCurrent} />
{/if}

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
    flex-wrap: wrap;
    gap: 4px 6px;
    padding: 6px 6px 4px;
  }
  .group {
    display: inline-flex;
    flex: none;
    gap: 1px;
  }
  .head :global(svg) {
    width: 15px;
    height: 15px;
  }
  .spacer {
    flex: 1;
  }
  .head button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 3px 4px;
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
  .sftp {
    position: relative;
  }
  .sftp.dropping {
    outline: 2px dashed var(--accent);
    outline-offset: -3px;
  }
  .drop-hint {
    position: absolute;
    inset: auto 8px 8px 8px;
    padding: 8px;
    border-radius: 4px;
    background: var(--accent);
    color: #fff;
    font-size: 12px;
    text-align: center;
    pointer-events: none;
    word-break: break-all;
  }
  .menu {
    position: fixed;
    z-index: 60;
    min-width: 190px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.5);
  }
  .menu button {
    text-align: left;
    border: none;
    background: transparent;
  }
  .menu button:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .menu hr {
    border: none;
    border-top: 1px solid var(--border);
    margin: 3px 0;
  }
  .menu kbd {
    float: right;
    margin-left: 18px;
    color: var(--fg-dim);
    font-size: 11px;
    font-family: inherit;
  }
  .menu .danger-item {
    color: var(--danger);
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
