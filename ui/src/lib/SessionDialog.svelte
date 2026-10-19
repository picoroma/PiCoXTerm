<script lang="ts">
  import { untrack } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { onMount } from 'svelte'
  import { testSession, type TestResult } from './api'
  import { homeDir, join } from '@tauri-apps/api/path'
  import { open } from '@tauri-apps/plugin-dialog'
  import Modal from './Modal.svelte'
  import { t } from '../i18n'
  import { THEME_NAMES, type ThemeName } from './themes'
  import type { AuthMethod, Session } from './types'

  let {
    session,
    isNew,
    candidates,
    onsave,
    oncancel,
  }: {
    session: Session
    isNew: boolean
    /** Sessioni SSH salvate usabili come jump host. */
    candidates: Session[]
    onsave: (s: Session, password: string | null, clearPassword: boolean) => void
    oncancel: () => void
  } = $props()

  // Copia di lavoro: la sessione originale non viene toccata finché non si salva.
  const initial = untrack(() => $state.snapshot(session))
  let name = $state(initial.name)
  let host = $state(initial.host)
  let port = $state(initial.port)
  let username = $state(initial.username)
  let method = $state<AuthMethod>(initial.auth.method)
  let keyPath = $state(initial.auth.keyPath ?? '')
  let secret = $state('')
  let clearSecret = $state(false)
  let notes = $state(initial.notes)
  let tags = $state(initial.ui.tags.join(', '))
  let uiTheme = $state<ThemeName | ''>(initial.ui.theme ?? '')
  let uiFontFamily = $state(initial.ui.fontFamily ?? '')
  let uiFontSize = $state<number | null>(initial.ui.fontSize)
  let bgOn = $state(initial.ui.background !== null)
  let bgColor = $state(initial.ui.background ?? '#2a0000')
  let colorOn = $state(initial.ui.color !== null)
  let tabColor = $state(initial.ui.color ?? '#c0392b')
  let jumps = $state<string[]>([...initial.jumpHosts])
  let jumpToAdd = $state('')
  const available = $derived(
    candidates.filter((c) => c.id !== initial.id && c.protocol === 'ssh' && !jumps.includes(c.id)),
  )
  const jumpName = (id: string) => candidates.find((c) => c.id === id)?.name ?? `(${t('missingSession')})`
  const hasStoredSecret = initial.credentialRef !== null
  /** Una sessione SFTP non ha terminale: niente tema, font e sfondo; ha invece una cartella iniziale. */
  const isFtp = initial.protocol === 'ftp'
  const isS3 = initial.protocol === 's3'
  /** Sessione che apre un file browser (SFTP o FTP): niente terminale, tema e font. */
  const isRdp = initial.protocol === 'rdp'
  const isSftp = initial.protocol === 'sftp' || isFtp || isS3 || isRdp
  let rdpDomain = $state(initial.rdp.domain ?? '')
  let rdpClient = $state(initial.rdp.client)
  let rdpFullscreen = $state(initial.rdp.fullscreen)
  let rdpMultimon = $state(initial.rdp.multimon)
  let rdpResolution = $state(initial.rdp.resolution ?? '')
  let rdpClipboard = $state(initial.rdp.redirectClipboard)
  let rdpDrives = $state(initial.rdp.redirectDrives)
  let rdpGateway = $state(initial.rdp.gateway ?? '')
  let s3Region = $state(initial.s3.region)
  let s3Endpoint = $state(initial.s3.endpoint ?? '')
  let s3PathStyle = $state(initial.s3.pathStyle)
  let s3Bucket = $state(initial.s3.bucket ?? '')
  let s3Profile = $state(initial.s3.profile ?? '')
  let startPath = $state((isFtp ? initial.ftp.startPath : initial.sftp.startPath) ?? '')
  let showHidden = $state(isFtp ? initial.ftp.showHidden : initial.sftp.showHidden)
  let ftpSecurity = $state(initial.ftp.security)
  let pinnedCert = $state(initial.ftp.pinnedCert)

  // La policy dell'organizzazione può vietare l'FTP in chiaro.
  let plainAllowed = $state(true)
  onMount(() => {
    invoke<{ policy: { allowPlainFtp: boolean } }>('policy_get')
      .then((r) => (plainAllowed = r.policy.allowPlainFtp))
      .catch(() => {})
  })
  const DEFAULT_PORT = { plain: 21, explicit: 21, implicit: 990 } as const
  /** Cambiando il tipo si passa alla porta abituale, ma solo se non è stata scelta a mano. */
  function onSecurityChange(next: 'plain' | 'explicit' | 'implicit') {
    if (port === DEFAULT_PORT[ftpSecurity]) port = DEFAULT_PORT[next]
    ftpSecurity = next
  }

  const usesSecret = $derived(isFtp || isRdp || (isS3 && s3Profile.trim() === '') || (!isS3 && (method === 'password' || method === 'key')))
  const valid = $derived(
    isS3
      ? s3Region.trim() !== '' && (s3Profile.trim() !== '' || username.trim() !== '')
      : isRdp
        ? host.trim() !== '' && Number.isInteger(port) && port >= 1 && port <= 65535
        : host.trim() !== '' &&
          username.trim() !== '' &&
          Number.isInteger(port) &&
          port >= 1 &&
          port <= 65535 &&
          (isFtp || method !== 'key' || keyPath.trim() !== ''),
  )

  /** Per S3 `host` e `port` servono solo a riconoscere la sessione nell'elenco. */
  function s3Host(): string {
    const e = s3Endpoint.trim()
    if (!e) return 's3.amazonaws.com'
    try {
      return new URL(e).host
    } catch {
      return e
    }
  }

  /** Cartella da cui aprire il selettore: quella della chiave attuale, altrimenti `~/.ssh`. */
  async function startFolder(): Promise<string | undefined> {
    const current = keyPath.trim()
    try {
      const home = await homeDir()
      if (!current) return await join(home, '.ssh')
      // `~` iniziale -> cartella home
      const full = current.startsWith('~') ? await join(home, current.slice(1).replace(/^[\\/]+/, '')) : current
      // tiene solo la cartella: il file potrebbe non esistere più
      const cut = Math.max(full.lastIndexOf('/'), full.lastIndexOf('\\'))
      return cut > 0 ? full.slice(0, cut) : await join(home, '.ssh')
    } catch {
      return undefined
    }
  }

  async function browse() {
    const defaultPath = await startFolder()
    const picked = await open({ multiple: false, directory: false, defaultPath, title: t('keyFile') })
    if (typeof picked === 'string') keyPath = picked
  }

  function addJump() {
    if (jumpToAdd && !jumps.includes(jumpToAdd)) jumps = [...jumps, jumpToAdd]
    jumpToAdd = ''
  }

  function moveJump(i: number, delta: number) {
    const j = i + delta
    if (j < 0 || j >= jumps.length) return
    const next = [...jumps]
    ;[next[i], next[j]] = [next[j], next[i]]
    jumps = next
  }

  /** La sessione com'è ora nel dialogo, anche se non ancora salvata. */
  function draft(): Session {
    return {
      ...initial,
      name: name.trim() || (isS3 ? (s3Bucket.trim() ? `s3://${s3Bucket.trim()}` : s3Host()) : host.trim()),
      host: isS3 ? s3Host() : host.trim(),
      port: isS3 ? 443 : port,
      username: username.trim(),
      auth: { method: isFtp || isS3 || isRdp ? 'password' : method, keyPath: !isFtp && !isS3 && !isRdp && method === 'key' ? keyPath.trim() : null },
      jumpHosts: $state.snapshot(jumps),
      sftp: initial.protocol === 'sftp' ? { startPath: startPath.trim() || null, showHidden } : initial.sftp,
      ftp: isFtp
        ? { security: ftpSecurity, pinnedCert: pinnedCert, startPath: startPath.trim() || null, showHidden }
        : initial.ftp,
      rdp: isRdp
        ? {
            domain: rdpDomain.trim() || null,
            client: rdpClient,
            fullscreen: rdpFullscreen,
            multimon: rdpMultimon,
            resolution: rdpResolution.trim() || null,
            redirectClipboard: rdpClipboard,
            redirectDrives: rdpDrives,
            gateway: rdpGateway.trim() || null,
          }
        : initial.rdp,
      s3: isS3
        ? {
            region: s3Region.trim(),
            endpoint: s3Endpoint.trim() || null,
            pathStyle: s3PathStyle,
            bucket: s3Bucket.trim() || null,
            profile: s3Profile.trim() || null,
            showHidden: initial.s3.showHidden,
          }
        : initial.s3,
      notes,
      ui: {
        ...initial.ui,
        tags: tags
          .split(',')
          .map((x) => x.trim())
          .filter(Boolean),
        theme: uiTheme || null,
        fontFamily: uiFontFamily.trim() || null,
        fontSize: uiFontSize ? Number(uiFontSize) : null,
        background: bgOn ? bgColor : null,
        color: colorOn ? tabColor : null,
      },
      updatedAt: new Date().toISOString(),
    }
  }

  let testing = $state(false)
  let testResult = $state<TestResult | null>(null)

  async function runTest() {
    testing = true
    testResult = null
    try {
      testResult = await testSession(draft(), secret === '' ? null : secret)
    } catch (e) {
      testResult = { status: 'error', message: String(e), elapsedMs: null, keyType: null, fingerprint: null }
    } finally {
      testing = false
    }
  }

  function submit(e: Event) {
    e.preventDefault()
    if (!valid) return
    onsave(draft(), secret === '' ? null : secret, clearSecret && secret === '')
  }
</script>

<Modal
  title={isRdp ? (isNew ? t('newRdpSession') : t('editRdpSession')) : isS3 ? (isNew ? t('newS3Session') : t('editS3Session')) : isFtp ? (isNew ? t('newFtpSession') : t('editFtpSession')) : isSftp ? (isNew ? t('newSftpSession') : t('editSftpSession')) : isNew ? t('newSession') : t('editSession')}
  onclose={oncancel}
>
  <form id="session-form" onsubmit={submit}>
    <label>{t('name')}<input bind:value={name} placeholder={isS3 ? s3Host() : host} /></label>
    {#if isS3}
      <div class="row">
        <label class="grow">{t('s3Region')}<input bind:value={s3Region} required spellcheck="false" /></label>
      </div>
      <label>
        {t('s3Endpoint')}
        <input bind:value={s3Endpoint} placeholder="https://s3.example.com" spellcheck="false" />
      </label>
      <p class="hint">{t('s3EndpointHint')}</p>
      <label class="check"><input type="checkbox" bind:checked={s3PathStyle} /> {t('s3PathStyle')}</label>
      <label>{t('s3Bucket')}<input bind:value={s3Bucket} spellcheck="false" /></label>
      <p class="hint">{t('s3BucketHint')}</p>
      <label>{t('s3Profile')}<input bind:value={s3Profile} spellcheck="false" /></label>
      <p class="hint">{t('s3ProfileHint')}</p>
      {#if s3Profile.trim() === ''}
        <label>{t('s3AccessKey')}<input bind:value={username} autocomplete="off" spellcheck="false" /></label>
      {/if}
    {:else}
    <div class="row">
      <label class="grow">{t('host')}<input bind:value={host} required /></label>
      <label class="narrow">{t('port')}<input type="number" min="1" max="65535" bind:value={port} required /></label>
    </div>
    <label>{t('user')}<input bind:value={username} required autocomplete="off" /></label>
    {/if}
    {#if isRdp}
      <label>{t('rdpDomain')}<input bind:value={rdpDomain} autocomplete="off" spellcheck="false" /></label>
    {/if}
    {#if isFtp}
      <label>
        {t('ftpSecurity')}
        <select value={ftpSecurity} onchange={(e) => onSecurityChange(e.currentTarget.value as 'plain' | 'explicit' | 'implicit')}>
          <option value="explicit">{t('ftpExplicit')}</option>
          <option value="implicit">{t('ftpImplicit')}</option>
          {#if plainAllowed || ftpSecurity === 'plain'}
            <option value="plain" disabled={!plainAllowed}>{t('ftpPlain')}</option>
          {/if}
        </select>
      </label>
      {#if ftpSecurity === 'plain'}
        <p class="hint warn" role="status">{plainAllowed ? t('plainFtpWarning') : t('plainFtpForbidden')}</p>
      {:else if pinnedCert}
        <div class="hint pin">
          <span>{t('ftpPinned')}: <code>{pinnedCert}</code></span>
          <button type="button" onclick={() => (pinnedCert = null)}>{t('ftpForgetCert')}</button>
        </div>
      {/if}
    {:else if !isS3 && !isRdp}
    <label>
      {t('authMethod')}
      <select bind:value={method}>
        <option value="password">{t('methodPassword')}</option>
        <option value="key">{t('methodKey')}</option>
        <option value="agent">{t('methodAgent')}</option>
      </select>
    </label>
    {/if}
    {#if method === 'key'}
      <label>
        {t('keyFile')}
        <span class="row">
          <input class="grow" bind:value={keyPath} placeholder="~/.ssh/id_ed25519" required />
          <button type="button" onclick={browse}>{t('browse')}</button>
        </span>
      </label>
    {/if}
    {#if method === 'agent'}
      <p class="hint">{t('agentHint')}</p>
    {/if}
    {#if usesSecret}
      <label>
        {isS3 ? t('s3SecretKey') : method === 'key' ? t('passphrase') : t('password')}
        <input
          type="password"
          bind:value={secret}
          autocomplete="new-password"
          placeholder={hasStoredSecret
            ? t('secretSaved')
            : isS3
              ? t('s3SecretNone')
              : method === 'key'
              ? t('passphraseNone')
              : t('passwordNone')}
        />
      </label>
    {/if}
    {#if hasStoredSecret}
      <label class="check"><input type="checkbox" bind:checked={clearSecret} /> {t('clearPassword')}</label>
    {/if}
    {#if isRdp}
      <fieldset>
        <legend>RDP</legend>
        <label>
          {t('rdpClient')}
          <select bind:value={rdpClient}>
            <option value="auto">{t('rdpClientAuto')}</option>
            <option value="mstsc">mstsc (Windows)</option>
            <option value="windows-app">Windows App (macOS)</option>
            <option value="freerdp">FreeRDP</option>
          </select>
        </label>
        <label>{t('rdpResolution')}<input bind:value={rdpResolution} placeholder="1920x1080" spellcheck="false" /></label>
        <label>{t('rdpGateway')}<input bind:value={rdpGateway} placeholder="gateway.example.com" spellcheck="false" /></label>
        <label class="check"><input type="checkbox" bind:checked={rdpFullscreen} /> {t('rdpFullscreen')}</label>
        <label class="check"><input type="checkbox" bind:checked={rdpMultimon} /> {t('rdpMultimon')}</label>
        <label class="check"><input type="checkbox" bind:checked={rdpClipboard} /> {t('rdpClipboard')}</label>
        <label class="check"><input type="checkbox" bind:checked={rdpDrives} /> {t('rdpDrives')}</label>
        {#if rdpDrives}<p class="hint warn" role="status">{t('rdpDrivesWarn')}</p>{/if}
        <p class="hint">{t('rdpPasswordHint')}</p>
      </fieldset>
    {/if}
    {#if !isFtp && !isS3 && !isRdp}
    <fieldset>
      <legend>{t('jumpHosts')}</legend>
      {#each jumps as id, i (id)}
        <div class="jump">
          <span class="grow">{i + 1}. {jumpName(id)}</span>
          <button type="button" aria-label="↑" disabled={i === 0} onclick={() => moveJump(i, -1)}>↑</button>
          <button type="button" aria-label="↓" disabled={i === jumps.length - 1} onclick={() => moveJump(i, 1)}>↓</button>
          <button type="button" aria-label={t('remove')} onclick={() => (jumps = jumps.filter((x) => x !== id))}>✕</button>
        </div>
      {/each}
      <div class="row">
        <select class="grow" bind:value={jumpToAdd} aria-label={t('jumpHosts')}>
          <option value="">{t('addJump')}</option>
          {#each available as c (c.id)}
            <option value={c.id}>{c.name} ({c.username}@{c.host})</option>
          {/each}
        </select>
        <button type="button" disabled={!jumpToAdd} onclick={addJump}>{t('add')}</button>
      </div>
      {#if jumps.length > 0}<p class="hint">{t('jumpHint')}</p>{/if}
    </fieldset>
    {/if}
    {#if isSftp && !isS3 && !isRdp}
      <fieldset>
        <legend>{isFtp ? 'FTP' : t('sftp')}</legend>
        <label>
          {t('sftpStartPath')}
          <input bind:value={startPath} placeholder="/home/utente" spellcheck="false" />
        </label>
        <label class="check"><input type="checkbox" bind:checked={showHidden} /> {t('showHidden')}</label>
      </fieldset>
    {/if}
    <fieldset>
      <legend>{t('appearance')}</legend>
      {#if !isSftp}
      <label>
        {t('theme')}
        <select bind:value={uiTheme}>
          <option value="">{t('themeDefault')}</option>
          {#each THEME_NAMES as name (name)}
            <option value={name}>{t(`theme_${name}`)}</option>
          {/each}
        </select>
      </label>
      <div class="row">
        <label class="grow">{t('fontFamily')}<input bind:value={uiFontFamily} placeholder={t('inherit')} /></label>
        <label class="narrow">{t('fontSize')}<input type="number" min="8" max="32" bind:value={uiFontSize} placeholder="—" /></label>
      </div>
      {/if}
      <div class="row center">
        {#if !isSftp}
        <label class="check"><input type="checkbox" bind:checked={bgOn} /> {t('customBackground')}</label>
        <input type="color" bind:value={bgColor} disabled={!bgOn} aria-label={t('customBackground')} />
        {/if}
        <label class="check"><input type="checkbox" bind:checked={colorOn} /> {t('tabColor')}</label>
        <input type="color" bind:value={tabColor} disabled={!colorOn} aria-label={t('tabColor')} />
      </div>
    </fieldset>
    <label>{t('tags')}<input bind:value={tags} placeholder="PRD, SAP" /></label>
    <label>{t('notes')}<textarea rows="3" bind:value={notes}></textarea></label>
  </form>
  {#if testResult}
    <div class="test {testResult.status}" role="status">
      <strong>
        {#if testResult.status === 'ok'}✓ {t('testOk', { ms: testResult.elapsedMs ?? 0 })}
        {:else if testResult.status === 'unknownHostKey'}ⓘ {t('testUnknownKey')}
        {:else if testResult.status === 'needSecret'}ⓘ {t('testNeedSecret')}
        {:else if testResult.status === 'hostKeyChanged'}⚠ {t('testKeyChanged')}
        {:else}✕ {t('testFailed')}{/if}
      </strong>
      <div>{testResult.message}</div>
      {#if testResult.fingerprint}<div class="mono">{testResult.keyType} {testResult.fingerprint}</div>{/if}
    </div>
  {/if}
  {#snippet actions()}
    <button type="button" class="left" disabled={!valid || testing} onclick={runTest}>
      {testing ? t('testing') : t('testConnection')}
    </button>
    <button type="button" onclick={oncancel}>{t('cancel')}</button>
    <button type="submit" form="session-form" class="primary" disabled={!valid}>{t('save')}</button>
  {/snippet}
</Modal>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
    color: var(--fg-dim);
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  .row {
    display: flex;
    gap: 10px;
  }
  .grow {
    flex: 1;
  }
  .narrow {
    width: 90px;
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: 4px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
    color: var(--fg-dim);
    font-size: 13px;
  }
  .jump {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--fg);
  }
  .jump button {
    padding: 2px 8px;
  }
  .center {
    align-items: center;
    flex-wrap: wrap;
  }
  input[type='color'] {
    padding: 0 2px;
    height: 28px;
    width: 44px;
  }
  .test {
    margin-top: 12px;
    padding: 8px 10px;
    border-radius: 4px;
    border: 1px solid var(--border);
    font-size: 13px;
    word-break: break-word;
  }
  .test.ok {
    border-color: #5cb85c;
  }
  .test.unknownHostKey,
  .test.needSecret {
    border-color: #f0ad4e;
  }
  .test.error,
  .test.hostKeyChanged {
    border-color: var(--danger);
  }
  .test .mono {
    font-family: Consolas, monospace;
    font-size: 12px;
    margin-top: 4px;
  }
  .left {
    margin-right: auto;
  }
  .hint {
    margin: 0;
    font-size: 13px;
    color: var(--fg-dim);
  }
  .hint.warn {
    padding: 6px 8px;
    border: 1px solid #f0ad4e;
    border-radius: 4px;
    color: var(--fg);
  }
  .hint.pin {
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: space-between;
  }
  .hint.pin code {
    font-size: 11px;
    word-break: break-all;
  }
</style>
