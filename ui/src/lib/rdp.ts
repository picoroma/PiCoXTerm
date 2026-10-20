// Tastiera dell'RDP: dal tasto fisico (`KeyboardEvent.code`) allo scancode RDP/XT. I tasti "estesi"
// hanno 0xE000 nei bit alti. Si usa il tasto fisico, non il carattere: il layout lo applica il server.

const PLAIN: Record<string, number> = {
  Escape: 0x01,
  Digit1: 0x02,
  Digit2: 0x03,
  Digit3: 0x04,
  Digit4: 0x05,
  Digit5: 0x06,
  Digit6: 0x07,
  Digit7: 0x08,
  Digit8: 0x09,
  Digit9: 0x0a,
  Digit0: 0x0b,
  Minus: 0x0c,
  Equal: 0x0d,
  Backspace: 0x0e,
  Tab: 0x0f,
  KeyQ: 0x10,
  KeyW: 0x11,
  KeyE: 0x12,
  KeyR: 0x13,
  KeyT: 0x14,
  KeyY: 0x15,
  KeyU: 0x16,
  KeyI: 0x17,
  KeyO: 0x18,
  KeyP: 0x19,
  BracketLeft: 0x1a,
  BracketRight: 0x1b,
  Enter: 0x1c,
  ControlLeft: 0x1d,
  KeyA: 0x1e,
  KeyS: 0x1f,
  KeyD: 0x20,
  KeyF: 0x21,
  KeyG: 0x22,
  KeyH: 0x23,
  KeyJ: 0x24,
  KeyK: 0x25,
  KeyL: 0x26,
  Semicolon: 0x27,
  Quote: 0x28,
  Backquote: 0x29,
  ShiftLeft: 0x2a,
  Backslash: 0x2b,
  KeyZ: 0x2c,
  KeyX: 0x2d,
  KeyC: 0x2e,
  KeyV: 0x2f,
  KeyB: 0x30,
  KeyN: 0x31,
  KeyM: 0x32,
  Comma: 0x33,
  Period: 0x34,
  Slash: 0x35,
  ShiftRight: 0x36,
  NumpadMultiply: 0x37,
  AltLeft: 0x38,
  Space: 0x39,
  CapsLock: 0x3a,
  F1: 0x3b,
  F2: 0x3c,
  F3: 0x3d,
  F4: 0x3e,
  F5: 0x3f,
  F6: 0x40,
  F7: 0x41,
  F8: 0x42,
  F9: 0x43,
  F10: 0x44,
  NumLock: 0x45,
  ScrollLock: 0x46,
  Numpad7: 0x47,
  Numpad8: 0x48,
  Numpad9: 0x49,
  NumpadSubtract: 0x4a,
  Numpad4: 0x4b,
  Numpad5: 0x4c,
  Numpad6: 0x4d,
  NumpadAdd: 0x4e,
  Numpad1: 0x4f,
  Numpad2: 0x50,
  Numpad3: 0x51,
  Numpad0: 0x52,
  NumpadDecimal: 0x53,
  IntlBackslash: 0x56, // il tasto "<>" delle tastiere ISO (italiana)
  F11: 0x57,
  F12: 0x58,
  IntlRo: 0x73,
  IntlYen: 0x7d,
}

const EXTENDED: Record<string, number> = {
  NumpadEnter: 0xe01c,
  ControlRight: 0xe01d,
  NumpadDivide: 0xe035,
  PrintScreen: 0xe037,
  AltRight: 0xe038, // AltGr: serve per @ # [ ] { } sulle tastiere italiane
  Home: 0xe047,
  ArrowUp: 0xe048,
  PageUp: 0xe049,
  ArrowLeft: 0xe04b,
  ArrowRight: 0xe04d,
  End: 0xe04f,
  ArrowDown: 0xe050,
  PageDown: 0xe051,
  Insert: 0xe052,
  Delete: 0xe053,
  MetaLeft: 0xe05b,
  MetaRight: 0xe05c,
  ContextMenu: 0xe05d,
}

/** Scancode RDP del tasto, o `null` se non è gestito. */
export function scancodeOf(code: string): number | null {
  return PLAIN[code] ?? EXTENDED[code] ?? null
}

/** Ctrl+Alt+Canc: sequenza di tasti da mandare al server (il sistema locale la intercetterebbe). */
export const CTRL_ALT_DEL: { scancode: number; pressed: boolean }[] = [
  { scancode: 0x1d, pressed: true },
  { scancode: 0x38, pressed: true },
  { scancode: 0xe053, pressed: true },
  { scancode: 0xe053, pressed: false },
  { scancode: 0x38, pressed: false },
  { scancode: 0x1d, pressed: false },
]

/** Messaggi binari: grafica `[1][x][y][w][h][rgba]` (interi a 16 bit little-endian), fine `[2][motivo]`, testo copiato nel server `[3][utf-8]`, nuova risoluzione `[4][w][h]`. */
export type RdpFrame =
  | { kind: 'update'; x: number; y: number; width: number; height: number; rgba: Uint8ClampedArray }
  | { kind: 'disconnected'; reason: string }
  | { kind: 'clipboard'; text: string }
  | { kind: 'resized'; width: number; height: number }
  | null

export function parseFrame(buf: ArrayBuffer | number[]): RdpFrame {
  const u8 = buf instanceof ArrayBuffer ? new Uint8Array(buf) : Uint8Array.from(buf)
  if (u8.length === 0) return null
  if (u8[0] === 4 && u8.length === 5) {
    const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength)
    return { kind: 'resized', width: dv.getUint16(1, true), height: dv.getUint16(3, true) }
  }
  if (u8[0] === 3) return { kind: 'clipboard', text: new TextDecoder().decode(u8.subarray(1)) }
  if (u8[0] === 2) return { kind: 'disconnected', reason: new TextDecoder().decode(u8.subarray(1)) }
  if (u8[0] === 1 && u8.length >= 9) {
    const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength)
    const [x, y, width, height] = [dv.getUint16(1, true), dv.getUint16(3, true), dv.getUint16(5, true), dv.getUint16(7, true)]
    if (u8.length - 9 !== width * height * 4) return null
    return { kind: 'update', x, y, width, height, rgba: new Uint8ClampedArray(u8.buffer, u8.byteOffset + 9, width * height * 4) }
  }
  return null
}
