// UNTAB: schede in finestre proprie. La connessione resta nel backend (indicizzata per numero): la finestra
// nuova la riprende, e si può riagganciare alla principale senza ricollegarsi.

import { invoke } from '@tauri-apps/api/core'

/** Cosa mostra una finestra staccata (tipo = protocollo della sessione). */
export type DetachedKind = 'sftp' | 'ftp' | 's3' | 'rdp'

export interface DetachedInfo {
  kind: DetachedKind
  connId: number
  sessionId: string | null
  title: string
  extra: unknown
  /** Etichetta della finestra (`detached-<tipo>-<connessione>`). */
  label: string
}

/** Dati della finestra in cui gira questa pagina, se è una scheda staccata (il backend li passa all'avvio). */
export function currentDetached(): DetachedInfo | null {
  return (window as unknown as { __DETACHED__?: DetachedInfo }).__DETACHED__ ?? null
}

/**
 * Connessioni che devono sopravvivere alla chiusura della scheda che le possedeva (perché la scheda sta
 * passando a un'altra finestra): le schede, quando spariscono, chiudono la loro connessione, salvo queste.
 */
export const handoff = new Set<number>()

/** Chiude la connessione di una scheda staccata (RDP o file). */
export function closeConnection(kind: DetachedKind, connId: number): Promise<unknown> {
  return kind === 'rdp' ? invoke('rdp_close', { id: connId }) : invoke('ssh_close', { id: connId })
}
