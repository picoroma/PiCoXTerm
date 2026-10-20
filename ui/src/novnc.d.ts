declare module '@novnc/novnc' {
  export default class RFB extends EventTarget {
    constructor(target: HTMLElement, urlOrChannel: string | WebSocket, options?: Record<string, unknown>)
    scaleViewport: boolean
    resizeSession: boolean
    clipViewport: boolean
    viewOnly: boolean
    focus(): void
    disconnect(): void
    sendCtrlAltDel(): void
    clipboardPasteFrom(text: string): void
  }
}
