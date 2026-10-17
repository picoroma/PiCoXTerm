// Specchio del modello di pxt-core (sezione 4 delle specifiche). Qui non ci sono mai segreti.

import type { ThemeName } from './themes'

export type Uuid = string

export interface CredentialRef {
  provider: string
  id: string
}

/** Aspetto per sessione: i campi null ereditano dalle impostazioni globali. */
export interface SessionUi {
  color: string | null
  icon: string | null
  tags: string[]
  theme: ThemeName | null
  fontFamily: string | null
  fontSize: number | null
  background: string | null
}

export type AuthMethod = 'password' | 'key' | 'agent' | 'keyboard-interactive'

export interface Folder {
  type: 'folder'
  id: Uuid
  name: string
  parentId: Uuid | null
}

export interface Session {
  type: 'session'
  id: Uuid
  parentId: Uuid | null
  name: string
  protocol: 'ssh' | 'sftp' | 'ftp'
  host: string
  port: number
  username: string
  credentialRef: CredentialRef | null
  auth: { method: AuthMethod; keyPath: string | null }
  jumpHosts: Uuid[]
  ssh: {
    followCwd: 'auto' | 'osc7-only' | 'off'
    keepAliveSecs: number
    x11Forwarding: boolean
    initialCommand: string | null
    terminalType: string
    encoding: string
  }
  sftp: { startPath: string | null; showHidden: boolean }
  ftp: { security: 'plain' | 'explicit' | 'implicit'; pinnedCert: string | null; startPath: string | null; showHidden: boolean }
  ui: SessionUi
  notes: string
  createdAt: string
  updatedAt: string
}

export type SessionNode = Folder | Session

/** Cosa apre una scheda terminale. */
export interface Target {
  /** Sessione salvata da aprire. Se assente: quick connect con host/port/username. */
  sessionId?: Uuid
  host?: string
  port?: number
  username?: string
}

export function newFolder(name: string, parentId: Uuid | null): Folder {
  return { type: 'folder', id: crypto.randomUUID(), name, parentId }
}

export function newSession(parentId: Uuid | null, protocol: Session['protocol'] = 'ssh'): Session {
  const now = new Date().toISOString()
  return {
    type: 'session',
    id: crypto.randomUUID(),
    parentId,
    name: '',
    protocol,
    host: '',
    port: 22,
    username: '',
    credentialRef: null,
    auth: { method: 'password', keyPath: null },
    jumpHosts: [],
    ssh: {
      followCwd: 'auto',
      keepAliveSecs: 30,
      x11Forwarding: false,
      initialCommand: null,
      terminalType: 'xterm-256color',
      encoding: 'utf-8',
    },
    sftp: { startPath: null, showHidden: false },
    ftp: { security: 'explicit', pinnedCert: null, startPath: null, showHidden: false },
    ui: { color: null, icon: null, tags: [], theme: null, fontFamily: null, fontSize: null, background: null },
    notes: '',
    createdAt: now,
    updatedAt: now,
  }
}

/** Id del nodo e di tutti i suoi discendenti. */
export function subtreeIds(nodes: SessionNode[], root: Uuid): Set<Uuid> {
  const ids = new Set<Uuid>([root])
  let grew = true
  while (grew) {
    grew = false
    for (const n of nodes) {
      if (n.parentId && ids.has(n.parentId) && !ids.has(n.id)) {
        ids.add(n.id)
        grew = true
      }
    }
  }
  return ids
}

/** Voce di una cartella remota (specchio di `pxt_fs::Entry`). */
export interface SftpEntry {
  name: string
  path: string
  kind: 'file' | 'dir' | 'other'
  isSymlink: boolean
  size: number
  modified: number | null
  mode: number | null
  owner: string | null
  group: string | null
  uid: number | null
  gid: number | null
}

/** File di testo aperto nell'editor integrato (il contenuto usa sempre `\n`). */
export interface TextFile {
  content: string
  encoding: 'utf8' | 'utf8-bom' | 'latin1'
  eol: 'lf' | 'crlf'
  mixedEol: boolean
  size: number
  modified: number | null
}

export type SaveOutcome =
  | { status: 'saved'; size: number; modified: number | null }
  | { status: 'conflict'; size: number | null; modified: number | null; missing: boolean }

/** Stato del tracciamento della directory del terminale (OSC 7) di una scheda. */
export interface TrackingInfo {
  /** `pending` finché non si sa, poi `active`, `unsupported` (shell non gestita), `off` o `error`. */
  state: 'pending' | 'active' | 'unsupported' | 'off' | 'error'
  shell: string | null
  message: string | null
}
