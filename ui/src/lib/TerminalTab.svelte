<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'
  import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager'
  import { Terminal } from '@xterm/xterm'
  import { FitAddon } from '@xterm/addon-fit'
  import '@xterm/xterm/css/xterm.css'
  import Modal from './Modal.svelte'
  import { t } from '../i18n'

  import type { Target } from './types'

  type TermEvent =
    | { type: 'hostKey'; reqId: number; host: string; port: number; keyType: string; fingerprint: string }
    | { type: 'closed'; exit: number | null }
    | { type: 'cwd'; path: string }

  let {
    target,
    active,
    savePassword,
  }: {
    target: Target
    active: boolean
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
  let term: Terminal
  let fit: FitAddon
  let connId: number | null = null
  const encoder = new TextEncoder()

  async function copySelection() {
    const sel = term?.getSelection()
    if (sel) await writeText(sel)
  }

  async function pasteFromClipboard() {
    const text = await readText().catch(() => '')
    if (!text) return
    if (text.trim().includes('\n')) pendingPaste = text
    else term.paste(text)
  }

  function confirmPaste(ok: boolean) {
    const text = pendingPaste
    pendingPaste = null
    if (ok && text) term.paste(text)
    term.focus()
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
      fontFamily: 'Consolas, "DejaVu Sans Mono", monospace',
      fontSize: 14,
      scrollback: 10000,
      cursorBlink: true,
    })
    fit = new FitAddon()
    term.loadAddon(fit)
    term.open(termEl)
    fit.fit()
    // Copia su selezione; Ctrl+Shift+C / Ctrl+Shift+V come alternativa da tastiera
    term.onSelectionChange(() => void copySelection())
    term.attachCustomKeyEventHandler((ev) => {
      if (ev.type === 'keydown' && ev.ctrlKey && ev.shiftKey) {
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
    void connect()

    return () => {
      obs.disconnect()
      if (connId !== null) void invoke('ssh_close', { id: connId })
      term.dispose()
    }
  })

  $effect(() => {
    if (active) void tick().then(() => term?.focus())
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

  <div
    class="term"
    role="presentation"
    bind:this={termEl}
    oncontextmenu={(e) => {
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
    background: #000;
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
