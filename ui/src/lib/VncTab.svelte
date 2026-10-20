<script lang="ts">
  import { onMount } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'
  import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager'
  import RFB from '@novnc/novnc'
  import ConnectPrompts from './ConnectPrompts.svelte'
  import { t } from '../i18n'
  import type { Target } from './types'

  type TermEvent = { type: 'vncClosed'; reason: string } | { type: string }

  let {
    target,
    active,
    savePassword,
    viewOnly = false,
    shareClipboard = true,
    connId = $bindable(null),
    status = $bindable('connecting'),
  }: {
    target: Target
    active: boolean
    savePassword?: (password: string) => Promise<void>
    viewOnly?: boolean
    /** Gli appunti (solo testo) sono condivisi con il server per questa sessione. */
    shareClipboard?: boolean
    connId?: number | null
    status?: 'connecting' | 'connected' | 'closed'
  } = $props()

  const PASSWORD_REQUIRED = 'password-required'

  /**
   * Il "canale" che noVNC crede sia una WebSocket: i byte viaggiano per IPC verso la connessione TCP aperta
   * dal backend (nessuna porta in ascolto). noVNC controlla che esistano queste proprietà e usa gli stati
   * come stringhe (`connecting`, `open`, `closed`).
   */
  class IpcSocket {
    binaryType = 'arraybuffer'
    protocol = ''
    readyState = 'connecting'
    onopen: (() => void) | null = null
    onmessage: ((e: { data: ArrayBuffer }) => void) | null = null
    onclose: ((e?: unknown) => void) | null = null
    onerror: ((e?: unknown) => void) | null = null
    /** Byte arrivati prima che noVNC fosse pronto a riceverli. */
    queue: ArrayBuffer[] = []
    id: number | null = null
    chain: Promise<unknown> = Promise.resolve()

    deliver(buf: ArrayBuffer | number[]) {
      const ab = buf instanceof ArrayBuffer ? buf : Uint8Array.from(buf).buffer
      if (this.onmessage) this.onmessage({ data: ab })
      else this.queue.push(ab)
    }

    flush() {
      const q = this.queue
      this.queue = []
      for (const ab of q) this.onmessage?.({ data: ab })
    }

    send(data: Uint8Array) {
      const id = this.id
      if (id === null) return
      // Una coda sola: l'ordine dei byte deve restare quello di noVNC.
      const copy = new Uint8Array(data)
      this.chain = this.chain.then(() => invoke('vnc_send', copy, { headers: { 'x-conn-id': String(id) } })).catch(() => {})
    }

    close() {
      if (this.readyState === 'closed') return
      this.readyState = 'closed'
      const id = this.id
      if (id !== null) void invoke('vnc_close', { id }).catch(() => {})
      this.onclose?.({})
    }
  }

  let screen = $state<HTMLDivElement>()
  let error = $state('')
  let endReason = $state('')
  let askPassword = $state(false)
  let saveRequested = $state(false)
  let autoResize = $state(true)
  let fit = $state(true)
  let desktopName = $state('')
  let rfb: InstanceType<typeof RFB> | null = null
  let sock: IpcSocket | null = null
  let lastClipboard = ''
  const MAX_CLIPBOARD = 4 * 1024 * 1024

  function teardown() {
    try {
      rfb?.disconnect()
    } catch {
      /* già chiuso */
    }
    rfb = null
    if (screen) screen.replaceChildren()
  }

  async function connect(password?: string) {
    error = ''
    endReason = ''
    status = 'connecting'
    teardown()
    const s = new IpcSocket()
    sock = s
    const onData = new Channel<ArrayBuffer | number[]>()
    onData.onmessage = (b) => s.deliver(b)
    const onEvent = new Channel<TermEvent>()
    onEvent.onmessage = (ev) => {
      if (ev.type === 'vncClosed') {
        endReason = (ev as { reason: string }).reason
        status = 'closed'
        connId = null
        s.readyState = 'closed'
        s.onclose?.({})
      }
    }
    try {
      const id = await invoke<number>('vnc_connect', {
        request: { sessionId: target.sessionId, password: password ?? null },
        onData,
        onEvent,
      })
      s.id = id
      connId = id
    } catch (e) {
      status = 'closed'
      if (String(e) === PASSWORD_REQUIRED) askPassword = true
      else error = String(e)
      return
    }
    if (!screen) return
    s.readyState = 'open'
    const r = new RFB(screen, s as unknown as WebSocket, { shared: true })
    rfb = r
    r.scaleViewport = fit
    r.resizeSession = autoResize
    r.viewOnly = viewOnly
    r.addEventListener('connect', () => {
      status = 'connected'
      r.focus()
    })
    r.addEventListener('disconnect', (e: Event) => {
      const clean = (e as CustomEvent<{ clean: boolean }>).detail?.clean
      if (status !== 'closed') {
        status = 'closed'
        if (!endReason) endReason = clean ? t('vncEndedClean') : t('vncEndedError')
      }
      connId = null
    })
    r.addEventListener('securityfailure', (e: Event) => {
      error = (e as CustomEvent<{ reason: string }>).detail?.reason ?? t('vncEndedError')
    })
    r.addEventListener('desktopname', (e: Event) => {
      desktopName = (e as CustomEvent<{ name: string }>).detail?.name ?? ''
    })
    r.addEventListener('clipboard', (e: Event) => {
      if (!shareClipboard) return
      const text = (e as CustomEvent<{ text: string }>).detail?.text
      if (text && text.length <= MAX_CLIPBOARD) {
        lastClipboard = text
        void writeText(text).catch(() => {})
      }
    })
    // I primi byte (il finto handshake) possono essere già arrivati: ora noVNC è pronto.
    setTimeout(() => s.flush(), 0)
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

  async function disconnect() {
    status = 'closed'
    teardown()
    connId = null
  }

  /** Il testo copiato sul computer diventa incollabile nel server; si legge solo quando la scheda prende il fuoco. */
  async function syncClipboard() {
    if (!rfb || !shareClipboard || status !== 'connected') return
    try {
      const text = await readText()
      if (!text || text === lastClipboard || text.length > MAX_CLIPBOARD) return
      lastClipboard = text
      rfb.clipboardPasteFrom(text)
    } catch {
      /* appunti non disponibili o con altro contenuto */
    }
  }

  async function toggleFullscreen() {
    try {
      if (document.fullscreenElement) await document.exitFullscreen()
      else await screen?.requestFullscreen()
    } catch {
      /* il sistema può non permetterlo */
    }
    rfb?.focus()
  }

  $effect(() => {
    if (rfb) rfb.scaleViewport = fit
  })
  $effect(() => {
    if (rfb) rfb.resizeSession = autoResize
  })

  onMount(() => {
    void connect()
    return () => {
      teardown()
      sock?.close()
    }
  })

  $effect(() => {
    if (active && status === 'connected') rfb?.focus()
  })
</script>

<div class="tab" class:hidden={!active}>
  <div class="tools">
    {#if status === 'connected'}
      <span class="dim">{t('vncConnected')}{desktopName ? ` — ${desktopName}` : ''}</span>
      <button type="button" onclick={() => rfb?.sendCtrlAltDel()} title={t('rdpCtrlAltDel')}>Ctrl+Alt+Canc</button>
      <button type="button" class:on={autoResize} aria-pressed={autoResize} title={t('rdpAutoResizeHint')} onclick={() => (autoResize = !autoResize)}>{t('rdpAutoResize')}</button>
      <button type="button" class:on={fit} aria-pressed={fit} onclick={() => (fit = !fit)}>{t('rdpFit')}</button>
      <button type="button" onclick={toggleFullscreen}>{t('rdpFullscreen')}</button>
      <button type="button" onclick={disconnect}>{t('disconnect')}</button>
    {:else if status === 'connecting'}
      <span class="dim">{t('connecting')}</span>
    {:else}
      <button type="button" class="primary" onclick={() => connect()}>{t('reconnect')}</button>
    {/if}
  </div>
  {#if status === 'connected'}<div class="warn" role="status">{t('vncPlainWarning')}</div>{/if}
  {#if error}<div class="error">{error}</div>{/if}
  {#if endReason && status === 'closed'}<div class="note">{t('rdpEnded')}: {endReason}</div>{/if}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="screen" bind:this={screen} onfocusin={syncClipboard}></div>
</div>

<ConnectPrompts
  hostKey={null}
  hostKeyChanged={null}
  {askPassword}
  askKind="password"
  canSave={!!target.sessionId && !!savePassword}
  onreplyhostkey={() => {}}
  ondismisshostkeychanged={() => {}}
  onsubmitpassword={submitPassword}
  oncancelpassword={() => (askPassword = false)}
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
  .tools button.on {
    outline: 1px solid var(--accent);
  }
  .dim {
    color: var(--fg-dim);
    margin-right: auto;
  }
  .warn {
    padding: 4px 8px;
    background: #5a3d1d;
    color: #ffe4c4;
    font-size: 12px;
  }
  .error {
    padding: 8px;
    background: #5a1d1d;
    color: #ffd6d6;
    white-space: pre-wrap;
  }
  .note {
    padding: 6px 8px;
    background: #5a3d1d;
    color: #ffe4c4;
    font-size: 13px;
  }
  .screen {
    flex: 1;
    min-height: 0;
    background: #000;
    overflow: hidden;
  }
</style>
