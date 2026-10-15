<script lang="ts">
  import { Channel, invoke } from '@tauri-apps/api/core'
  import { Terminal } from '@xterm/xterm'
  import { FitAddon } from '@xterm/addon-fit'
  import '@xterm/xterm/css/xterm.css'
  import { t } from './i18n'

  type TermEvent =
    | { type: 'hostKey'; reqId: number; host: string; port: number; keyType: string; fingerprint: string }
    | { type: 'closed'; exit: number | null }
    | { type: 'cwd'; path: string }

  let host = $state('')
  let port = $state(22)
  let user = $state('')
  let password = $state('')
  let status = $state<'idle' | 'connecting' | 'connected'>('idle')
  let error = $state('')
  let cwd = $state('')
  let injectShell = $state('bash')
  let hostKeyPrompt = $state<Extract<TermEvent, { type: 'hostKey' }> | null>(null)

  let termEl: HTMLDivElement
  let term: Terminal | null = null
  let fit: FitAddon | null = null
  let sessionId: number | null = null
  let resizeObs: ResizeObserver | null = null
  const encoder = new TextEncoder()

  function disposeTerm() {
    resizeObs?.disconnect()
    resizeObs = null
    term?.dispose()
    term = null
  }

  async function connect(e: Event) {
    e.preventDefault()
    error = ''
    status = 'connecting'
    disposeTerm()

    term = new Terminal({
      fontFamily: 'Consolas, "DejaVu Sans Mono", monospace',
      fontSize: 14,
      scrollback: 10000,
      cursorBlink: true,
    })
    fit = new FitAddon()
    term.loadAddon(fit)
    term.open(termEl)
    fit.fit()

    const onData = new Channel<ArrayBuffer | number[]>()
    onData.onmessage = (buf) => {
      term?.write(buf instanceof ArrayBuffer ? new Uint8Array(buf) : new Uint8Array(buf))
    }
    const onEvent = new Channel<TermEvent>()
    onEvent.onmessage = (ev) => {
      if (ev.type === 'hostKey') {
        hostKeyPrompt = ev
      } else if (ev.type === 'cwd') {
        cwd = ev.path
      } else if (ev.type === 'closed') {
        term?.write(`\r\n\x1b[33m[${t('closed')}]\x1b[0m\r\n`)
        status = 'idle'
        sessionId = null
      }
    }

    const pwd = password
    password = ''
    try {
      sessionId = await invoke<number>('ssh_connect', {
        host,
        port,
        username: user,
        password: pwd,
        cols: term.cols,
        rows: term.rows,
        onData,
        onEvent,
      })
    } catch (err) {
      error = String(err)
      status = 'idle'
      disposeTerm()
      return
    }
    status = 'connected'

    term.onData((s) => {
      if (sessionId !== null) invoke('ssh_write', { id: sessionId, data: Array.from(encoder.encode(s)) })
    })
    resizeObs = new ResizeObserver(() => {
      if (!term || !fit) return
      fit.fit()
      if (sessionId !== null) invoke('ssh_resize', { id: sessionId, cols: term.cols, rows: term.rows })
    })
    resizeObs.observe(termEl)
    term.focus()
  }

  async function replyHostKey(accept: boolean) {
    if (!hostKeyPrompt) return
    const reqId = hostKeyPrompt.reqId
    hostKeyPrompt = null
    await invoke('hostkey_reply', { reqId, accept })
  }

  async function injectCwd() {
    if (sessionId !== null) await invoke('ssh_inject_cwd', { id: sessionId, shell: injectShell })
    term?.focus()
  }

  async function disconnect() {
    if (sessionId !== null) await invoke('ssh_close', { id: sessionId })
  }
</script>

<main>
  <form class="bar" onsubmit={connect}>
    <input placeholder={t('host')} bind:value={host} required disabled={status !== 'idle'} />
    <input class="port" type="number" min="1" max="65535" aria-label={t('port')} bind:value={port} disabled={status !== 'idle'} />
    <input placeholder={t('user')} bind:value={user} required disabled={status !== 'idle'} />
    <input type="password" placeholder={t('password')} bind:value={password} disabled={status !== 'idle'} />
    {#if status === 'connected'}
      <select bind:value={injectShell} aria-label="shell">
        <option>bash</option>
        <option>tcsh</option>
      </select>
      <button type="button" onclick={injectCwd}>{t('injectCwd')}</button>
      <button type="button" onclick={disconnect}>{t('disconnect')}</button>
    {:else}
      <button type="submit" disabled={status === 'connecting'}>
        {status === 'connecting' ? t('connecting') : t('connect')}
      </button>
    {/if}
  </form>
  {#if error}<div class="error">{error}</div>{/if}

  <div class="term" bind:this={termEl}></div>
  <div class="status">{t('cwd')}: {cwd || '—'}</div>

  {#if hostKeyPrompt}
    <div class="backdrop">
      <div class="dialog" role="alertdialog" aria-labelledby="hk-title">
        <h2 id="hk-title">{t('hostKeyTitle')}</h2>
        <p>{t('hostKeyBody')}</p>
        <dl>
          <dt>{t('host')}</dt><dd>{hostKeyPrompt.host}:{hostKeyPrompt.port}</dd>
          <dt>{t('keyType')}</dt><dd>{hostKeyPrompt.keyType}</dd>
          <dt>{t('fingerprint')}</dt><dd class="mono">{hostKeyPrompt.fingerprint}</dd>
        </dl>
        <div class="actions">
          <button onclick={() => replyHostKey(false)}>{t('cancel')}</button>
          <button class="primary" onclick={() => replyHostKey(true)}>{t('accept')}</button>
        </div>
      </div>
    </div>
  {/if}
</main>

<style>
  :global(html, body) {
    margin: 0;
    height: 100%;
    background: #1e1e1e;
    color: #ddd;
    font-family: system-ui, sans-serif;
  }
  main {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  .bar {
    display: flex;
    gap: 8px;
    padding: 8px;
    background: #2b2b2b;
  }
  .status {
    padding: 4px 8px;
    background: #2b2b2b;
    font-family: Consolas, monospace;
    font-size: 13px;
  }
  select,
  input,
  button {
    padding: 6px 10px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #1e1e1e;
    color: #ddd;
  }
  input.port {
    width: 70px;
  }
  button {
    cursor: pointer;
  }
  button.primary {
    background: #2f6fed;
    border-color: #2f6fed;
    color: #fff;
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
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: grid;
    place-items: center;
  }
  .dialog {
    background: #2b2b2b;
    padding: 20px;
    border-radius: 8px;
    max-width: 520px;
  }
  .dialog h2 {
    margin-top: 0;
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
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
</style>
