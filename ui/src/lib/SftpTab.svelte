<script lang="ts">
  import { onMount } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'
  import ConnectPrompts, { type CertRequest, type HostKeyChange, type HostKeyRequest } from './ConnectPrompts.svelte'
  import DualPane from './DualPane.svelte'
  import S3Pane from './S3Pane.svelte'
  import { t } from '../i18n'
  import type { SftpEntry, Target } from './types'

  type TermEvent =
    | ({ type: 'hostKey' } & HostKeyRequest)
    | ({ type: 'hostKeyChanged' } & HostKeyChange)
    | ({ type: 'certUntrusted' } & CertRequest)
    | { type: 'closed'; exit: number | null }

  let {
    target,
    active,
    protocol = 'sftp',
    plainFtp = false,
    onpincert,
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
    /** `ftp`: sessione FTP o FTPS (comando `ftp_connect`, conferma del certificato). */
    protocol?: 'sftp' | 'ftp' | 's3'
    /** FTP senza cifratura: si avvisa che password e dati viaggiano in chiaro. */
    plainFtp?: boolean
    /** Salva nella sessione l'impronta del certificato che l'utente ha accettato. */
    onpincert?: (fingerprint: string) => Promise<void>
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
  const CERT_UNTRUSTED = 'certificate-untrusted'

  let error = $state('')
  let hostKeyPrompt = $state<HostKeyRequest | null>(null)
  let hostKeyChanged = $state<HostKeyChange | null>(null)
  let askPassword = $state(false)
  let askKind = $state<'password' | 'passphrase'>('password')
  let saveRequested = $state(false)
  let certPrompt = $state<CertRequest | null>(null)
  /** Password già digitata in questa apertura: serve a riprovare dopo la conferma del certificato. */
  let lastPassword: string | undefined

  async function connect(password?: string, pinnedCert?: string) {
    error = ''
    status = 'connecting'
    lastPassword = password
    const onEvent = new Channel<TermEvent>()
    onEvent.onmessage = (ev) => {
      if (ev.type === 'hostKey') hostKeyPrompt = ev
      else if (ev.type === 'hostKeyChanged') hostKeyChanged = ev
      else if (ev.type === 'certUntrusted') certPrompt = ev
    }
    try {
      connId =
        protocol === 's3'
          ? await invoke<number>('s3_connect', {
              request: { sessionId: target.sessionId, password: password ?? null },
            })
          : protocol === 'ftp'
          ? await invoke<number>('ftp_connect', {
              request: { sessionId: target.sessionId, password: password ?? null, pinnedCert: pinnedCert ?? null },
              onEvent,
            })
          : await invoke<number>('sftp_connect', {
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
      if (String(e) === CERT_UNTRUSTED) {
        // La finestra di conferma è già aperta (evento `certUntrusted`): niente messaggio d'errore.
        return
      }
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

  /** Risposta alla finestra del certificato: si riprova con l'impronta accettata. */
  async function replyCert(decision: 'once' | 'save' | 'reject') {
    const cert = certPrompt
    certPrompt = null
    if (!cert || decision === 'reject') {
      if (cert) error = t('certRejected')
      return
    }
    if (decision === 'save') {
      try {
        await onpincert?.(cert.fingerprint)
      } catch (e) {
        error = String(e)
        return
      }
    }
    await connect(lastPassword, cert.fingerprint)
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
      <span class="dim">{protocol === 's3' ? t('s3Connected') : protocol === 'ftp' ? t('ftpConnected') : t('sftpConnected')}</span>
      <button type="button" onclick={disconnect}>{t('disconnect')}</button>
    {:else if status === 'connecting'}
      <span class="dim">{t('connecting')}</span>
    {:else}
      <button type="button" class="primary" onclick={() => connect()}>{t('reconnect')}</button>
    {/if}
  </div>
  {#if plainFtp && status === 'connected'}
    <div class="warn" role="status">{t('plainFtpWarning')}</div>
  {/if}
  {#if error}<div class="error">{error}</div>{/if}

  <div class="panel">
    {#if connId !== null && protocol === 's3'}
      <S3Pane {connId} {startPath} {showHidden} {sessionName} {sessionColor} />
    {:else if connId !== null}
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
  {certPrompt}
  oncertreply={replyCert}
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
  .warn {
    padding: 6px 8px;
    background: #5a3d1d;
    color: #ffe4c4;
    font-size: 13px;
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
