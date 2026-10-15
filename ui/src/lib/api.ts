import { invoke } from '@tauri-apps/api/core'
import type { SessionNode, Uuid } from './types'

export const listNodes = () => invoke<SessionNode[]>('sessions_list')

/** `password`: valore da mettere nel keyring (null = invariata). */
export const saveNode = (node: SessionNode, password: string | null = null, clearPassword = false) =>
  invoke<SessionNode>('session_save', { node, password, clearPassword })

export const deleteNode = (id: Uuid) => invoke<void>('session_delete', { id })
