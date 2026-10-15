<script lang="ts">
  import { onMount } from 'svelte'
  import Modal from './lib/Modal.svelte'
  import SessionDialog from './lib/SessionDialog.svelte'
  import SessionTree from './lib/SessionTree.svelte'
  import TerminalTab from './lib/TerminalTab.svelte'
  import SettingsDialog from './lib/SettingsDialog.svelte'
  import { loadSettings } from './lib/settings.svelte'
  import { deleteNode, listNodes, saveNode } from './lib/api'
  import { newFolder, newSession, type Folder, type Session, type SessionNode, type Target, type Uuid } from './lib/types'
  import { t } from './i18n'

  interface Tab {
    key: number
    title: string
    target: Target
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
    const tab = { key: nextKey++, title, target }
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

<main>
  <aside>
    <SessionTree
      {nodes}
      onopen={openSession}
      onedit={(s) => (editing = { session: $state.snapshot(s) as Session, isNew: false })}
      onnewsession={startNewSession}
      onnewfolder={startNewFolder}
      onrename={startRename}
      ondelete={(n) => (deleting = n)}
      onmove={move}
    />
  </aside>

  <section class="content">
    <div class="topbar">
      <form onsubmit={quickConnect}>
        <input bind:value={quick} placeholder={t('quickConnect')} aria-label={t('quickConnect')} />
        <button type="submit">{t('connect')}</button>
      </form>
      <button type="button" class="gear" title={t('settings')} aria-label={t('settings')} onclick={() => (showSettings = true)}>⚙</button>
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
          savePassword={tab.target.sessionId ? passwordSaver(tab.target.sessionId) : undefined}
        />
      {/each}
      {#if tabs.length === 0}
        <p class="welcome">{t('welcome')}</p>
      {/if}
    </div>
  </section>
</main>

{#if showSettings}
  <SettingsDialog onclose={() => (showSettings = false)} />
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
    grid-template-columns: 300px 1fr;
    height: 100vh;
  }
  aside {
    background: var(--bg-elev);
    border-right: 1px solid var(--border);
    min-height: 0;
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
