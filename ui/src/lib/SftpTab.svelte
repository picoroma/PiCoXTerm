<script lang="ts">
  import { onMount } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'
  import ConnectPrompts, { type HostKeyChange, type HostKeyRequest } from './ConnectPrompts.svelte'
  import DualPane from './DualPane.svelte'
  import { t } from '../i18n'
  import type { SftpEntry, Target } from './types'

  type TermEvent =
    | ({ type: 'hostKey' } & HostKeyRequest)
    | ({ type: 'hostKeyChanged' } & HostKeyChange)
    | { type: 'closed'; exit: number | null }

  let {
    target,
    active,
    startPath = null,
    showHidden = false,
    sessionName = '',
    sessionColor = null,
    savePassword,
    onmanagehostkeys,
    onedit,
    onexternal,
    connId = $bindable(null),
    status = $bindable('connecting'),
  }: {
    target: Target
    active: boolean
    /** Cartella remota da aprire alla connessione (vuoto = home). */
    startPath?: string | null
    showHidden?: boolean
    sessionName?: string
    sessionColor?: string | null
    savePassword?: (password: string) => Promise<void>
    onmanagehostkeys?: () => void
    onedit?: (connId: number, entry: SftpEntry) => void
    onexternal?: (connId: number, entry: SftpEntry) => void
    connId?: number | null
    status?: 'connecting' | 'connected' | 'closed'
  } = $props()

  const PASSWORD_REQUIRED = 'password-required'
  const PASSPHRASE_REQUIRED = 'passphrase-required'
  const HOSTKEY_CHANGED = 'hostkey-changed'

  let error = $state('')
  let hostKeyPrompt = $state<HostKeyRequest | null>(null)
  let hostKeyChanged = $state<HostKeyChange | null>(null)
  let askPassword = $state(false)
  let askKind = $state<'password' | 'passphrase'>('password')
  let saveRequested = $state(false)

  async function connect(password?: string) {
    error = ''
    status = 'connecting'
    const onEvent = new Channel<TermEvent>()
    onEvent.onmessage = (ev) => {
      if (ev.type === 'hostKey') hostKeyPrompt = ev
      else if (ev.type === 'hostKeyChanged') hostKeyChanged = ev
    }
    try {
      connId = await invoke<number>('sftp_connect', {
        request: {
          sessionId: target.sessionId ?? null,
          host: target.host ?? null,
          port: target.port ?? null,
          username: target.username ?? null,
          password: password ?? null,
        },
        onEvent,
      })
    } catch (e) {
      status = 'closed'
      if (String(e) === PASSWORD_REQUIRED || String(e) === PASSPHRASE_REQUIRED) {
        askKind = String(e) === PASSPHRASE_REQUIRED ? 'passphrase' : 'password'
        askPassword = true
      } else if (String(e) === HOSTKEY_CHANGED) {
        error = t('hostKeyChangedShort')
      } else {
        error = String(e)
      }
      return
    }
    status = 'connected'
    if (password && saveRequested && savePassword) {
      await savePassword(password).catch((e) => (error = String(e)))
    }
    saveRequested = false
  }

  async function submitPassword(password: string, save: boolean) {
    askPassword = false
    saveRequested = save
    await connect(password)
  }

  async function replyHostKey(decision: 'once' | 'save' | 'reject') {
    if (!hostKeyPrompt) return
    const reqId = hostKeyPrompt.reqId
    hostKeyPrompt = null
    await invoke('hostkey_reply', { reqId, decision })
  }

  async function disconnect() {
    const id = connId
    connId = null
    status = 'closed'
    if (id !== null) await invoke('ssh_close', { id })
  }

  onMount(() => {
    void connect()
    return () => {
      if (connId !== null) void invoke('ssh_close', { id: connId })
    }
  })
</script>

<div class="tab" class:hidden={!active}>
  <div class="tools">
    {#if status === 'connected'}
      <span class="dim">{t('sftpConnected')}</span>
      <button type="button" onclick={disconnect}>{t('disconnect')}</button>
    {:else if status === 'connecting'}
      <span class="dim">{t('connecting')}</span>
    {:else}
      <button type="button" class="primary" onclick={() => connect()}>{t('reconnect')}</button>
    {/if}
  </div>
  {#if error}<div class="error">{error}</div>{/if}

  <div class="panel">
    {#if connId !== null}
      <DualPane
        {connId}
        {startPath}
        {showHidden}
        {sessionName}
        {sessionColor}
        {onedit}
        {onexternal}
      />
    {/if}
  </div>
</div>

<ConnectPrompts
  hostKey={hostKeyPrompt}
  {hostKeyChanged}
  {askPassword}
  {askKind}
  canSave={!!target.sessionId && !!savePassword}
  onreplyhostkey={replyHostKey}
  ondismisshostkeychanged={() => (hostKeyChanged = null)}
  onsubmitpassword={submitPassword}
  oncancelpassword={() => (askPassword = false)}
  {onmanagehostkeys}
/>

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
  .panel {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
</style>
