<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'
  import DualPane from './DualPane.svelte'
  import Modal from './Modal.svelte'
  import RdpTab from './RdpTab.svelte'
  import S3Pane from './S3Pane.svelte'
  import TerminalTab from './TerminalTab.svelte'
  import TransferBar from './TransferBar.svelte'
  import { closeConnection, handoff, type DetachedInfo } from './detach'
  import { listNodes } from './api'
  import { loadSettings } from './settings.svelte'
  import { t } from '../i18n'
  import type { Session } from './types'

  let { info }: { info: DetachedInfo } = $props()

  let session = $state<Session | null>(null)
  let askClose = $state(false)
  let status = $state<'connecting' | 'connected' | 'closed'>('connecting')
  let connId = $state<number | null>(null)
  $effect.pre(() => {
    connId ??= info.connId
  })

  onMount(() => {
    document.title = info.title
    void loadSettings().catch(() => {})
    void listNodes()
      .then((nodes) => {
        const n = nodes.find((x) => x.id === info.sessionId)
        if (n?.type === 'session') session = n
      })
      .catch(() => {})
    // Chiudendo la finestra (la X) il backend la trattiene e chiede cosa fare.
    const unlisten = listen('detached-close-requested', () => (askClose = true))
    return () => void unlisten.then((f) => f())
  })

  const files = $derived(info.kind === 'sftp' || info.kind === 'ftp' || info.kind === 's3')
  const startPath = $derived(
    info.kind === 's3' && session?.s3.bucket ? `/${session.s3.bucket}` : null,
  )
  const showHidden = $derived(
    session ? (info.kind === 's3' ? session.s3.showHidden : info.kind === 'ftp' ? session.ftp.showHidden : session.sftp.showHidden) : false,
  )
  const color = $derived(session?.ui.color ?? null)

  function finish(action: 'redock' | 'ended') {
    if (action === 'redock') handoff.add(info.connId)
    void invoke('window_finish', { label: info.label, action, info })
  }

  async function closeSession() {
    await closeConnection(info.kind, info.connId).catch(() => {})
    finish('ended')
  }
</script>

<div class="detached">
  <header>
    <strong title={info.title}>{info.title}</strong>
    <span class="dim">{t('detachedHint')}</span>
    <button type="button" onclick={() => finish('redock')}>{t('detachRedock')}</button>
    <button type="button" class="danger" onclick={closeSession}>{t('detachCloseSession')}</button>
  </header>
  <div class="body">
    {#if files && connId !== null}
      {#if info.kind === 's3'}
        <S3Pane {connId} {startPath} {showHidden} sessionName={info.title} sessionColor={color} />
      {:else}
        <DualPane {connId} {showHidden} sessionName={info.title} sessionColor={color} />
      {/if}
    {:else if info.kind === 'ssh'}
      <TerminalTab
        target={{ sessionId: info.sessionId ?? undefined }}
        active={true}
        ui={session?.ui ?? null}
        attachConnId={info.connId}
        bind:connId
        bind:status
      />
    {:else if info.kind === 'rdp'}
      <RdpTab
        target={{ sessionId: info.sessionId ?? undefined }}
        active={true}
        attachConnId={info.connId}
        shareClipboard={session?.rdp.redirectClipboard ?? true}
        bind:connId
        bind:status
      />
    {/if}
  </div>
  <TransferBar />
</div>

{#if askClose}
  <Modal title={t('detachCloseTitle')} onclose={() => (askClose = false)}>
    <p>{t('detachCloseBody')}</p>
    {#snippet actions()}
      <button type="button" onclick={() => (askClose = false)}>{t('cancel')}</button>
      <button type="button" class="danger" onclick={closeSession}>{t('detachCloseSession')}</button>
      <button type="button" class="primary" onclick={() => finish('redock')}>{t('detachRedock')}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .detached {
    display: flex;
    flex-direction: column;
    height: 100vh;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 10px;
    background: var(--bg-elev);
    border-bottom: 1px solid var(--border);
  }
  header strong {
    max-width: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    color: var(--fg-dim);
    font-size: 12px;
    margin-right: auto;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .danger {
    color: var(--danger);
  }
</style>
