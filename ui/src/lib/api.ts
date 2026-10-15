import { invoke } from '@tauri-apps/api/core'
import type { Session, SessionNode, Uuid } from './types'

export interface TestResult {
  status: 'ok' | 'needSecret' | 'unknownHostKey' | 'hostKeyChanged' | 'error'
  message: string
  elapsedMs: number | null
  keyType: string | null
  fingerprint: string | null
}

export const listNodes = () => invoke<SessionNode[]>('sessions_list')

/** `password`: valore da mettere nel keyring (null = invariata). */
export const saveNode = (node: SessionNode, password: string | null = null, clearPassword = false) =>
  invoke<SessionNode>('session_save', { node, password, clearPassword })

export const deleteNode = (id: Uuid) => invoke<void>('session_delete', { id })

/** Prova una sessione (anche non salvata) senza aprire la shell. */
export const testSession = (session: Session, password: string | null) =>
  invoke<TestResult>('session_test', { session, password })

export const duplicateSession = (id: Uuid, copySuffix: string) =>
  invoke<SessionNode>('session_duplicate', { id, copySuffix })
