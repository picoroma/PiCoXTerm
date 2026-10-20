<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'
  import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager'
  import ConnectPrompts, { type CertRequest } from './ConnectPrompts.svelte'
  import { CTRL_ALT_DEL, parseFrame, scancodeOf, type RdpFrame } from './rdp'
  import { t } from '../i18n'
  import type { Target } from './types'

  type TermEvent = ({ type: 'certUntrusted' } & CertRequest) | { type: string }

  let {
    target,
    active,
    onpincert,
    savePassword,
    shareClipboard = true,
    connId = $bindable(null),
    status = $bindable('connecting'),
  }: {
    target: Target
    active: boolean
    /** Salva nella sessione l'impronta del certificato che l'utente ha accettato. */
    onpincert?: (fingerprint: string) => Promise<void>
    savePassword?: (password: string) => Promise<void>
    /** Gli appunti (solo testo) sono condivisi con il server per questa sessione. */
    shareClipboard?: boolean
    connId?: number | null
    status?: 'connecting' | 'connected' | 'closed'
  } = $props()

  const PASSWORD_REQUIRED = 'password-required'
  const CERT_UNTRUSTED = 'certificate-untrusted'

  let wrap = $state<HTMLDivElement>()
  let canvas = $state<HTMLCanvasElement>()
  let error = $state('')
  let endReason = $state('')
  let askPassword = $state(false)
  let saveRequested = $state(false)
  let certPrompt = $state<CertRequest | null>(null)
  let lastPassword: string | undefined
  let desktop = $state({ width: 0, height: 0 })
  let fit = $state(true)
  /** Area disponibile nella scheda: il desktop si adatta (anche ingrandendo) quando la finestra cambia. */
  let area = $state({ width: 0, height: 0 })
  const shown = $derived.by(() => {
    if (!fit || !desktop.width || !area.width || !area.height) return null
    const scale = Math.min(area.width / desktop.width, area.height / desktop.height)
    return { width: Math.max(1, Math.floor(desktop.width * scale)), height: Math.max(1, Math.floor(desktop.height * scale)) }
  })
  let ctx: CanvasRenderingContext2D | null = null
  /** Immagini arrivate prima che il canvas fosse pronto: si disegnano appena possibile. */
  let early: RdpFrame[] = []

  function paint(f: NonNullable<RdpFrame>) {
    if (f.kind !== 'update' || !ctx) return
    ctx.putImageData(new ImageData(f.rgba as Uint8ClampedArray<ArrayBuffer>, f.width, f.height), f.x, f.y)
  }

  /** Dimensione del desktop richiesto: l'area della scheda, a multipli di 4. */
  function areaSize(): { width: number; height: number } {
    const w = Math.max(640, Math.floor((wrap?.clientWidth ?? 1024) / 4) * 4)
    const h = Math.max(480, Math.floor((wrap?.clientHeight ?? 768) / 4) * 4)
    return { width: Math.min(w, 4096), height: Math.min(h, 4096) }
  }

  async function connect(password?: string, pinnedCert?: string) {
    error = ''
    endReason = ''
    status = 'connecting'
    lastPassword = password
    ctx = null
    early = []
    const onEvent = new Channel<TermEvent>()
    onEvent.onmessage = (ev) => {
      if (ev.type === 'certUntrusted') certPrompt = ev as { type: 'certUntrusted' } & CertRequest
    }
    const onFrame = new Channel<ArrayBuffer | number[]>()
    onFrame.onmessage = (buf) => {
      const f = parseFrame(buf)
      if (!f) return
      if (f.kind === 'clipboard') {
        // Testo copiato nel server: finisce negli appunti del computer (e non si rimanda indietro).
        lastClipboard = f.text
        void writeText(f.text).catch(() => {})
        return
      }
      if (f.kind === 'disconnected') {
        endReason = f.reason
        status = 'closed'
        connId = null
        return
      }
      if (ctx) paint(f)
      else early.push(f)
    }
    const size = areaSize()
    try {
      const r = await invoke<{ id: number; width: number; height: number }>('rdp_connect', {
        request: { sessionId: target.sessionId, password: password ?? null, pinnedCert: pinnedCert ?? null, ...size },
        onEvent,
        onFrame,
      })
      desktop = { width: r.width, height: r.height }
      connId = r.id
      status = 'connected'
      // Il canvas ha la dimensione del desktop; poi si disegna quanto è già arrivato.
      await tick()
      if (canvas) {
        canvas.width = r.width
        canvas.height = r.height
        ctx = canvas.getContext('2d', { alpha: false })
        ctx?.fillRect(0, 0, r.width, r.height)
        for (const f of early) if (f) paint(f)
        early = []
        canvas.focus()
      }
    } catch (e) {
      status = 'closed'
      const msg = String(e)
      if (msg === CERT_UNTRUSTED) return // la finestra di conferma è già aperta
      if (msg === PASSWORD_REQUIRED) askPassword = true
      else error = msg
      return
    }
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
    if (id !== null) await invoke('rdp_close', { id })
  }

  // ---- Appunti: solo testo ----
  /** Ultimo testo scambiato (in un verso o nell'altro): evita di rimandarlo indietro. */
  let lastClipboard = ''
  const MAX_CLIPBOARD = 4 * 1024 * 1024

  /** Il testo copiato sul computer diventa incollabile nel server. Si legge solo quando la scheda prende il fuoco. */
  async function syncClipboard() {
    const id = connId
    if (id === null || !shareClipboard) return
    try {
      const text = await readText()
      if (!text || text === lastClipboard || text.length > MAX_CLIPBOARD) return
      lastClipboard = text
      await invoke('rdp_clipboard_set', { id, text })
    } catch {
      /* appunti non disponibili o con altro contenuto (immagini): niente da fare */
    }
  }

  // ---- Input: tutto passa da una coda, così l'ordine di tasti e pulsanti è sempre rispettato ----
  let queue: Promise<unknown> = Promise.resolve()
  function send(input: Record<string, unknown>) {
    const id = connId
    if (id === null) return
    queue = queue.then(() => invoke('rdp_input', { id, input })).catch(() => {})
  }

  // I movimenti del mouse si riducono a uno per fotogramma.
  let pendingMove: { x: number; y: number } | null = null
  let moveScheduled = false
  function queueMove(x: number, y: number) {
    pendingMove = { x, y }
    if (moveScheduled) return
    moveScheduled = true
    requestAnimationFrame(() => {
      moveScheduled = false
      if (pendingMove) send({ kind: 'mouseMove', ...pendingMove })
      pendingMove = null
    })
  }

  /** Dalle coordinate della finestra a quelle del desktop remoto. */
  function toDesktop(e: MouseEvent): { x: number; y: number } | null {
    if (!canvas || !desktop.width) return null
    const r = canvas.getBoundingClientRect()
    if (r.width === 0 || r.height === 0) return null
    const x = Math.round(((e.clientX - r.left) * desktop.width) / r.width)
    const y = Math.round(((e.clientY - r.top) * desktop.height) / r.height)
    return { x: Math.min(desktop.width - 1, Math.max(0, x)), y: Math.min(desktop.height - 1, Math.max(0, y)) }
  }

  function onpointermove(e: PointerEvent) {
    const p = toDesktop(e)
    if (p) queueMove(p.x, p.y)
  }

  function onpointerdown(e: PointerEvent) {
    canvas?.focus()
    canvas?.setPointerCapture(e.pointerId)
    const p = toDesktop(e)
    if (p) send({ kind: 'mouseMove', ...p })
    send({ kind: 'mouseButton', button: Math.min(e.button, 4), pressed: true })
    e.preventDefault()
  }

  function onpointerup(e: PointerEvent) {
    const p = toDesktop(e)
    if (p) send({ kind: 'mouseMove', ...p })
    send({ kind: 'mouseButton', button: Math.min(e.button, 4), pressed: false })
    e.preventDefault()
  }

  function onwheel(e: WheelEvent) {
    e.preventDefault()
    // 120 unità per "tacca", come Windows; i pixel si convertono in modo proporzionale.
    const per = e.deltaMode === 1 ? 120 : e.deltaMode === 2 ? 360 : 1.2
    const units = Math.max(-32768, Math.min(32767, Math.round(-e.deltaY * per)))
    if (units !== 0) send({ kind: 'wheel', vertical: true, units })
    const h = Math.max(-32768, Math.min(32767, Math.round(e.deltaX * per)))
    if (h !== 0) send({ kind: 'wheel', vertical: false, units: h })
  }

  function key(e: KeyboardEvent, pressed: boolean) {
    const sc = scancodeOf(e.code)
    // L'applicazione non deve reagire ai tasti destinati al server (F1 apre le informazioni, per esempio).
    e.stopPropagation()
    if (sc === null) return
    e.preventDefault()
    send({ kind: 'key', scancode: sc, pressed })
  }

  function onblur() {
    // Un tasto rilasciato mentre la scheda non ha il fuoco resterebbe premuto sul server.
    send({ kind: 'releaseAll' })
  }

  function sendCtrlAltDel() {
    for (const k of CTRL_ALT_DEL) send({ kind: 'key', ...k })
    canvas?.focus()
  }

  async function toggleFullscreen() {
    try {
      if (document.fullscreenElement) await document.exitFullscreen()
      else await wrap?.requestFullscreen()
    } catch {
      /* il sistema può non permetterlo */
    }
    canvas?.focus()
  }

  onMount(() => {
    const ro = new ResizeObserver(() => {
      if (wrap) area = { width: wrap.clientWidth, height: wrap.clientHeight }
    })
    if (wrap) {
      ro.observe(wrap)
      area = { width: wrap.clientWidth, height: wrap.clientHeight }
    }
    void connect()
    return () => {
      ro.disconnect()
      if (connId !== null) void invoke('rdp_close', { id: connId })
    }
  })

  // Tornando sulla scheda la tastiera riparte dal desktop.
  $effect(() => {
    if (active && status === 'connected') canvas?.focus()
  })
</script>

<div class="tab" class:hidden={!active}>
  <div class="tools">
    {#if status === 'connected'}
      <span class="dim">{t('rdpConnected')}</span>
      <button type="button" onclick={sendCtrlAltDel} title={t('rdpCtrlAltDel')}>Ctrl+Alt+Canc</button>
      <button type="button" class:on={fit} aria-pressed={fit} onclick={() => (fit = !fit)}>{t('rdpFit')}</button>
      <button type="button" onclick={toggleFullscreen}>{t('rdpFullscreen')}</button>
      <button type="button" onclick={disconnect}>{t('disconnect')}</button>
    {:else if status === 'connecting'}
      <span class="dim">{t('connecting')}</span>
    {:else}
      <button type="button" class="primary" onclick={() => connect()}>{t('reconnect')}</button>
    {/if}
  </div>
  {#if error}<div class="error">{error}</div>{/if}
  {#if endReason && status === 'closed'}<div class="note">{t('rdpEnded')}: {endReason}</div>{/if}

  <div class="screen" bind:this={wrap}>
    {#if status === 'connected' || desktop.width > 0}
      <canvas
        bind:this={canvas}
        tabindex="0"
        class:hidden={status !== 'connected'}
        style:width={shown ? `${shown.width}px` : undefined}
        style:height={shown ? `${shown.height}px` : undefined}
        {onpointermove}
        {onpointerdown}
        {onpointerup}
        {onwheel}
        {onblur}
        onfocus={syncClipboard}
        onkeydown={(e) => key(e, true)}
        onkeyup={(e) => key(e, false)}
        oncontextmenu={(e) => e.preventDefault()}
      ></canvas>
    {/if}
  </div>
</div>

<ConnectPrompts
  hostKey={null}
  hostKeyChanged={null}
  {askPassword}
  askKind="password"
  canSave={!!target.sessionId && !!savePassword}
  {certPrompt}
  oncertreply={replyCert}
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
    overflow: auto;
    display: flex;
    background: #000;
  }
  canvas {
    /* margin automatico: centrato se più piccolo dell'area, scorrevole se più grande */
    margin: auto;
    flex: none;
    outline: none;
    cursor: default;
    background: #000;
    image-rendering: auto;
  }
  canvas.hidden {
    display: none;
  }
</style>
