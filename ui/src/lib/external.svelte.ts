import { Channel, invoke } from '@tauri-apps/api/core'
import type { SaveOutcome } from './types'

/** Un file remoto aperto con il programma di sistema: si segue la copia locale fino al salvataggio. */
export interface ExternalEdit {
  editId: number
  connId: number
  name: string
  path: string
  state: 'open' | 'changed' | 'uploading' | 'uploaded' | 'conflict' | 'missing' | 'error'
  /** Ricarica da solo ogni volta che il programma salva. */
  auto: boolean
  message: string
}

type ExternalEvent = { type: 'changed'; editId: number } | { type: 'closed'; editId: number }

export const externals: ExternalEdit[] = $state([])

function find(editId: number) {
  return externals.find((e) => e.editId === editId)
}

function remove(e: ExternalEdit) {
  const i = externals.indexOf(e)
  if (i >= 0) externals.splice(i, 1)
}

/** Scarica il file e lo apre con il programma di sistema. Restituisce il messaggio d'errore, se c'è. */
export async function openExternal(connId: number, entry: { name: string; path: string }): Promise<string | null> {
  const channel = new Channel<ExternalEvent>()
  channel.onmessage = (ev) => {
    const e = find(ev.editId)
    if (!e) return
    if (ev.type === 'closed') remove(e)
    else if (e.auto) void pushExternal(e, false)
    else e.state = 'changed'
  }
  try {
    const r = await invoke<{ editId: number; name: string; reused: boolean }>('external_edit_open', {
      id: connId,
      path: entry.path,
      onEvent: channel,
    })
    if (!find(r.editId)) {
      externals.push({ editId: r.editId, connId, name: entry.name, path: entry.path, state: 'open', auto: false, message: '' })
    }
    return null
  } catch (e) {
    return String(e)
  }
}

/** Ricarica la copia locale sul server. Con `force` sovrascrive anche se sul server è cambiato. */
export async function pushExternal(e: ExternalEdit, force: boolean): Promise<void> {
  e.state = 'uploading'
  e.message = ''
  try {
    const r = await invoke<SaveOutcome>('external_edit_push', { editId: e.editId, force })
    if (r.status === 'saved') e.state = 'uploaded'
    else e.state = r.missing ? 'missing' : 'conflict'
  } catch (err) {
    e.state = 'error'
    e.message = String(err)
  }
}

/** Smette di seguire il file ed elimina la copia temporanea. */
export async function stopExternal(e: ExternalEdit): Promise<void> {
  remove(e)
  try {
    await invoke('external_edit_stop', { editId: e.editId })
  } catch {
    /* la copia temporanea sparisce comunque alla chiusura della connessione o dell'app */
  }
}
