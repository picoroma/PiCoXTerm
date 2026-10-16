import type { IconName } from './Icon.svelte'

/** Tipi di sessione previsti dal progetto (sezione 2 delle specifiche). */
export type Protocol = 'ssh' | 'sftp' | 'ftp' | 's3' | 'rdp' | 'vnc'

export interface ProtocolInfo {
  id: Protocol
  icon: IconName
  /** Realizzato: gli altri compaiono nella finestra di scelta come "Presto". */
  available: boolean
}

/** Ordine di presentazione. Nomi e descrizioni sono in i18n (`proto_<id>`, `proto_<id>_desc`). */
export const PROTOCOLS: ProtocolInfo[] = [
  { id: 'ssh', icon: 'terminal', available: true },
  { id: 'sftp', icon: 'folder-transfer', available: false },
  { id: 'ftp', icon: 'swap', available: false },
  { id: 's3', icon: 'cloud', available: false },
  { id: 'rdp', icon: 'monitor', available: false },
  { id: 'vnc', icon: 'monitor-cursor', available: false },
]
