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
  protocol: 'ssh'
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

export function newSession(parentId: Uuid | null): Session {
  const now = new Date().toISOString()
  return {
    type: 'session',
    id: crypto.randomUUID(),
    parentId,
    name: '',
    protocol: 'ssh',
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
