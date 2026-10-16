<script lang="ts">
  import { onMount } from 'svelte'
  import Modal from './lib/Modal.svelte'
  import SessionDialog from './lib/SessionDialog.svelte'
  import SessionTree from './lib/SessionTree.svelte'
  import TerminalTab from './lib/TerminalTab.svelte'
  import SettingsDialog from './lib/SettingsDialog.svelte'
  import AboutDialog from './lib/AboutDialog.svelte'
  import Icon from './lib/Icon.svelte'
  import SftpPanel from './lib/SftpPanel.svelte'
  import TransferBar from './lib/TransferBar.svelte'
  import { loadSettings } from './lib/settings.svelte'
  import { deleteNode, duplicateSession, listNodes, saveNode } from './lib/api'
  import { invoke } from '@tauri-apps/api/core'
  import { newFolder, newSession, type TrackingInfo, type Folder, type Session, type SessionNode, type Target, type Uuid } from './lib/types'
  import { t } from './i18n'

  interface Tab {
    key: number
    title: string
    target: Target
    /** Collegamento SSH della scheda, per il pannello SFTP (aggiornato dalla scheda stessa). */
    connId: number | null
    cwd: string
    status: 'connecting' | 'connected' | 'closed'
    tracking: TrackingInfo
  }

  let nodes = $state<SessionNode[]>([])
  let tabs = $state<Tab[]>([])
  let activeKey = $state<number | null>(null)
  let nextKey = 1
  let notice = $state('')

  let editing = $state<{ session: Session; isNew: boolean } | null>(null)
  let folderDialog = $state<{ folder: Folder; isNew: boolean } | null>(null)
  let folderName = $state('')
  let deleting = $state<SessionNode | null>(null)
  let quick = $state('')
  let showSettings = $state(false)
  let showAbout = $state(false)

  const activeTab = $derived(tabs.find((x) => x.key === activeKey))
  const liveConnIds = $derived(tabs.map((x) => x.connId).filter((id): id is number => id !== null))

  // Dimensioni della sidebar e del pannello SFTP, ricordate tra un avvio e l'altro.
  const load = (key: string, fallback: number) => {
    try {
      const v = Number(localStorage.getItem(key))
      return Number.isFinite(v) && v > 0 ? v : fallback
    } catch {
      return fallback
    }
  }
  const store = (key: string, value: number) => {
    try {
      localStorage.setItem(key, String(Math.round(value)))
    } catch {
      /* il salvataggio è solo una comodità */
    }
  }
  let sidebarWidth = $state(load('pxt.sidebarWidth', 330))
  let sftpHeight = $state(load('pxt.sftpHeight', 300))
  let wide = $state(false)
  const NORMAL_WIDTH_MIN = 220
  const effectiveWidth = $derived(wide ? Math.min(Math.round(window.innerWidth * 0.6), 900) : sidebarWidth)

  /** Trascinamento di un divisore con il puntatore. */
  function drag(e: PointerEvent, kind: 'width' | 'height') {
    e.preventDefault()
    const startX = e.clientX
    const startY = e.clientY
    const startW = sidebarWidth
    const startH = sftpHeight
    const move = (ev: PointerEvent) => {
      if (kind === 'width') {
        wide = false
        sidebarWidth = Math.max(NORMAL_WIDTH_MIN, Math.min(700, startW + ev.clientX - startX))
      } else {
        sftpHeight = Math.max(140, Math.min(window.innerHeight - 220, startH - (ev.clientY - startY)))
      }
    }
    const up = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      store('pxt.sidebarWidth', sidebarWidth)
      store('pxt.sftpHeight', sftpHeight)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }

  async function reload() {
    try {
      nodes = await listNodes()
    } catch (e) {
      notice = `${t('loadError')}: ${e}`
    }
  }

  onMount(() => {
    void reload()
    loadSettings().catch((e) => (notice = String(e)))
  })

  function openTab(title: string, target: Target) {
    const tab: Tab = { key: nextKey++, title, target, connId: null, cwd: '', status: 'connecting', tracking: { state: 'pending', shell: null, message: null } }
    tabs.push(tab)
    activeKey = tab.key
  }

  function closeTab(key: number) {
    const i = tabs.findIndex((x) => x.key === key)
    if (i < 0) return
    tabs.splice(i, 1)
    if (activeKey === key) activeKey = tabs[Math.min(i, tabs.length - 1)]?.key ?? null
  }

  function openSession(s: Session) {
    openTab(s.name, { sessionId: s.id })
  }

  function quickConnect(e: Event) {
    e.preventDefault()
    const m = /^(?:([^@\s]+)@)?([^@:\s]+)(?::(\d{1,5}))?$/.exec(quick.trim())
    if (!m || !m[1]) {
      notice = t('quickConnectBad')
      return
    }
    notice = ''
    const [, username, host, port] = m
    openTab(`${username}@${host}`, { host, port: port ? Number(port) : 22, username })
    quick = ''
  }

  async function run(action: () => Promise<unknown>) {
    try {
      await action()
      notice = ''
    } catch (e) {
      notice = String(e)
    }
    await reload()
  }

  function startNewSession(parentId: Uuid | null) {
    editing = { session: newSession(parentId), isNew: true }
  }

  async function saveSession(s: Session, password: string | null, clearPassword: boolean) {
    editing = null
    await run(() => saveNode(s, password, clearPassword))
  }

  function startNewFolder(parentId: Uuid | null) {
    folderName = ''
    folderDialog = { folder: newFolder('', parentId), isNew: true }
  }

  function startRename(f: Folder) {
    folderName = f.name
    folderDialog = { folder: f, isNew: false }
  }

  async function saveFolder(e: Event) {
    e.preventDefault()
    if (!folderDialog || !folderName.trim()) return
    const folder = { ...folderDialog.folder, name: folderName.trim() }
    folderDialog = null
    await run(() => saveNode(folder))
  }

  async function confirmDelete() {
    const n = deleting
    deleting = null
    if (n) await run(() => deleteNode(n.id))
  }

  async function move(id: Uuid, parentId: Uuid | null) {
    const n = nodes.find((x) => x.id === id)
    if (!n) return
    const moved = { ...$state.snapshot(n), parentId } as SessionNode
    await run(() => saveNode(moved))
  }

  /** Attiva a mano il tracciamento della directory nella scheda attiva. */
  async function enableTracking() {
    const tab = activeTab
    if (!tab || tab.connId === null) return
    tab.tracking = { state: 'pending', shell: null, message: null }
    try {
      const r = await invoke<{ state: TrackingInfo['state']; shell: string | null; message: string | null }>(
        'ssh_enable_cwd_tracking',
        { id: tab.connId },
      )
      tab.tracking = { state: r.state, shell: r.shell, message: r.message }
    } catch (e) {
      tab.tracking = { state: 'error', shell: null, message: String(e) }
    }
  }

  /** Aspetto della sessione salvata (null per il quick connect). */
  function sessionUi(sessionId: Uuid | undefined) {
    const n = sessionId ? nodes.find((x) => x.id === sessionId) : undefined
    return n?.type === 'session' ? n.ui : null
  }

  /** Salva la password digitata in una scheda nella sessione corrispondente. */
  function passwordSaver(sessionId: Uuid) {
    return async (password: string) => {
      const n = nodes.find((x) => x.id === sessionId)
      if (n?.type === 'session') {
        await saveNode($state.snapshot(n) as Session, password, false)
        await reload()
      }
    }
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'F1') {
      e.preventDefault()
      showAbout = true
    }
  }}
/>

<main style:grid-template-columns="{effectiveWidth}px 5px 1fr">
  <aside>
    <div class="tree">
    <SessionTree
      {nodes}
      onopen={openSession}
      onedit={(s) => (editing = { session: $state.snapshot(s) as Session, isNew: false })}
      onduplicate={(s) => run(() => duplicateSession(s.id, t('copySuffix')))}
      onnewsession={startNewSession}
      onnewfolder={startNewFolder}
      onrename={startRename}
      ondelete={(n) => (deleting = n)}
      onmove={move}
    />
    </div>
    <div class="hsplit" role="separator" aria-orientation="horizontal" aria-label={t('resize')} onpointerdown={(e) => drag(e, 'height')}></div>
    <div class="sftp" style:height="{sftpHeight}px">
      <SftpPanel
        connId={activeTab?.status === 'connected' ? activeTab.connId : null}
        cwd={activeTab?.cwd ?? ''}
        liveIds={liveConnIds}
        tracking={activeTab?.tracking ?? { state: 'pending', shell: null, message: null }}
        onenabletracking={enableTracking}
        sessionName={activeTab?.title ?? ''}
        sessionColor={sessionUi(activeTab?.target.sessionId)?.color ?? null}
        {wide}
        onwiden={() => (wide = !wide)}
      />
    </div>
  </aside>
  <div class="vsplit" role="separator" aria-orientation="vertical" aria-label={t('resize')} onpointerdown={(e) => drag(e, 'width')}></div>

  <section class="content">
    <div class="topbar">
      <form onsubmit={quickConnect}>
        <input bind:value={quick} placeholder={t('quickConnect')} aria-label={t('quickConnect')} />
        <button type="submit">{t('connect')}</button>
      </form>
      <button type="button" class="gear" title={t('settings')} aria-label={t('settings')} onclick={() => (showSettings = true)}>⚙</button>
      <button type="button" class="info" title={t('aboutTitle')} aria-label={t('aboutTitle')} onclick={() => (showAbout = true)}><Icon name="info" /></button>
    </div>
    {#if notice}<div class="notice">{notice}</div>{/if}

    <div class="tabs" role="tablist">
      {#each tabs as tab (tab.key)}
        <div
          class="tab"
          class:on={tab.key === activeKey}
          style:border-top-color={sessionUi(tab.target.sessionId)?.color ?? undefined}
          role="presentation"
        >
          <button role="tab" aria-selected={tab.key === activeKey} onclick={() => (activeKey = tab.key)}>{tab.title}</button>
          <button class="x" aria-label={t('closeTab')} title={t('closeTab')} onclick={() => closeTab(tab.key)}>×</button>
        </div>
      {/each}
    </div>

    <div class="views">
      {#each tabs as tab (tab.key)}
        <TerminalTab
          target={tab.target}
          active={tab.key === activeKey}
          ui={sessionUi(tab.target.sessionId)}
          bind:connId={tab.connId}
          bind:cwd={tab.cwd}
          bind:status={tab.status}
          bind:tracking={tab.tracking}
          onmanagehostkeys={() => (showSettings = true)}
          savePassword={tab.target.sessionId ? passwordSaver(tab.target.sessionId) : undefined}
        />
      {/each}
      {#if tabs.length === 0}
        <p class="welcome">{t('welcome')}</p>
      {/if}
    </div>
    <TransferBar />
  </section>
</main>

{#if showAbout}
  <AboutDialog onclose={() => (showAbout = false)} />
{/if}

{#if showSettings}
  <SettingsDialog onclose={() => (showSettings = false)} onimported={reload} />
{/if}

{#if editing}
  <SessionDialog
    session={editing.session}
    isNew={editing.isNew}
    candidates={nodes.filter((n): n is Session => n.type === 'session')} onsave={saveSession} oncancel={() => (editing = null)} />
{/if}

{#if folderDialog}
  <Modal title={folderDialog.isNew ? t('newFolder') : t('rename')} onclose={() => (folderDialog = null)}>
    <form id="folder-form" onsubmit={saveFolder}>
      <input bind:value={folderName} placeholder={t('folderName')} aria-label={t('folderName')} required />
    </form>
    {#snippet actions()}
      <button type="button" onclick={() => (folderDialog = null)}>{t('cancel')}</button>
      <button type="submit" form="folder-form" class="primary">{t('save')}</button>
    {/snippet}
  </Modal>
{/if}

{#if deleting}
  <Modal title={t('delete')} onclose={() => (deleting = null)}>
    <p>{t(deleting.type === 'folder' ? 'confirmDeleteFolder' : 'confirmDeleteSession', { name: deleting.name })}</p>
    {#snippet actions()}
      <button type="button" onclick={() => (deleting = null)}>{t('cancel')}</button>
      <button type="button" class="danger" onclick={confirmDelete}>{t('delete')}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  main {
    display: grid;
    height: 100vh;
  }
  aside {
    background: var(--bg-elev);
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .tree {
    flex: 1 1 0;
    min-height: 120px;
    overflow: hidden;
  }
  .sftp {
    flex: none;
    min-height: 140px;
    overflow: hidden;
  }
  .hsplit {
    height: 5px;
    flex: none;
    cursor: row-resize;
    background: var(--border);
  }
  .vsplit {
    cursor: col-resize;
    background: var(--border);
  }
  .hsplit:hover,
  .vsplit:hover {
    background: var(--accent);
  }
  .content {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .topbar {
    padding: 8px;
    background: var(--bg-elev);
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .topbar .gear {
    margin-left: auto;
  }
  .topbar .info {
    display: inline-flex;
    align-items: center;
    padding: 6px 8px;
  }
  .topbar form {
    display: flex;
    gap: 8px;
    max-width: 520px;
  }
  .topbar input {
    flex: 1;
  }
  .notice {
    padding: 8px;
    background: #5a3d1d;
    color: #ffe4c4;
    white-space: pre-wrap;
  }
  .tabs {
    display: flex;
    gap: 2px;
    background: var(--bg-elev);
    padding: 0 8px;
    overflow-x: auto;
  }
  .tab {
    display: flex;
    align-items: center;
    border: 1px solid var(--border);
    border-bottom: none;
    border-top: 3px solid var(--border);
    border-radius: 6px 6px 0 0;
    background: var(--bg);
  }
  .tab.on {
    background: var(--bg-hover);
    border-color: var(--accent);
  }
  .tab button {
    border: none;
    background: transparent;
    padding: 5px 10px;
  }
  .tab .x {
    padding: 5px 8px;
    color: var(--fg-dim);
  }
  .views {
    flex: 1;
    min-height: 0;
  }
  .welcome {
    padding: 24px;
    color: var(--fg-dim);
  }
</style>
