// Backend finto per provare la UI in un normale browser (`npm run dev`). Solo in sviluppo:
// main.ts lo carica con un import dinamico dietro `import.meta.env.DEV`, quindi non finisce nell'exe.

import { newFolder, newSession, subtreeIds, type SessionNode } from '../lib/types'

const root = newFolder('Clienti', null)
const pw = { ...newSession(root.id), name: 'app01 (password)', host: '10.0.0.1', username: 'root' }
pw.credentialRef = { provider: 'local', id: 'x' }
pw.ui.tags = ['PRD']
pw.ui.color = '#c0392b'
pw.ui.background = '#2a0000'
pw.ui.theme = 'dracula'
pw.ui.fontSize = 16
const key = { ...newSession(null), name: 'jump (chiave)', host: '10.0.0.2', username: 'adm' }
key.auth = { method: 'key', keyPath: 'C:\\keys\\id_ed25519' }
let nodes: SessionNode[] = [root, pw, key]

let cb = 0
let settings: any = {
  schemaVersion: 1,
  theme: 'dark',
  fontFamily: 'Consolas, monospace',
  fontSize: 14,
  scrollback: 10000,
  copyOnSelect: true,
  rightClickPaste: true,
  pasteMultilineWarning: true,
}
const handlers: Record<string, (args: any) => unknown> = {
  settings_get: () => settings,
  settings_set: (args) => (settings = args.settings),
  sessions_list: () => JSON.parse(JSON.stringify(nodes)),
  session_save: ({ node }) => {
    const i = nodes.findIndex((n) => n.id === node.id)
    if (i >= 0) nodes[i] = node
    else nodes.push(node)
    return node
  },
  session_delete: ({ id }) => {
    const doomed = subtreeIds(nodes, id)
    nodes = nodes.filter((n) => !doomed.has(n.id))
  },
  ssh_connect: () => Promise.reject('password-required'),
  'plugin:path|resolve_directory': () => 'C:\\Users\\mock',
  'plugin:path|join': ({ paths }) => paths.join('\\'),
  'plugin:dialog|open': () => null,
}

;(window as any).__TAURI_INTERNALS__ = {
  invoke: async (cmd: string, args: any) => {
    const h = handlers[cmd]
    if (!h) throw new Error(`mock: comando non gestito ${cmd}`)
    // Come l'IPC reale: gli argomenti passano per JSON
    return h(args === undefined ? args : JSON.parse(JSON.stringify(args)))
  },
  transformCallback: () => ++cb,
  unregisterCallback: () => {},
  convertFileSrc: (p: string) => p,
  metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
}
