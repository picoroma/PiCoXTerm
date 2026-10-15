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

let nextCallback = 0
const callbacks = new Map<number, (m: unknown) => void>()
const channelIndex = new Map<number, number>()
let pendingReply: ((decision: string) => void) | null = null
;(window as any).__lastDecision = null

/** Invia un messaggio a un Channel ricevuto come argomento (`__CHANNEL__:<id>`). */
function emit(channel: string, message: unknown) {
  const id = Number(channel.split(':')[1])
  const i = channelIndex.get(id) ?? 0
  channelIndex.set(id, i + 1)
  callbacks.get(id)?.({ index: i, message })
}
// Server fittizio "srv" con un piccolo filesystem, per provare il pannello SFTP.
const d = (name: string, parent: string, extra: object = {}) => ({ name, path: `${parent === '/' ? '' : parent}/${name}`, kind: 'dir', isSymlink: false, size: 4096, modified: 1760000000, mode: 0o755, owner: 'u', group: 'users', uid: 1000, gid: 100, ...extra })
const f = (name: string, parent: string, size: number, extra: object = {}) => ({ name, path: `${parent === '/' ? '' : parent}/${name}`, kind: 'file', isSymlink: false, size, modified: 1760100000, mode: 0o644, owner: 'u', group: 'users', uid: 1000, gid: 100, ...extra })
const fakeFs: Record<string, any[]> = {
  '/': [d('home', '/'), d('etc', '/'), d('usr', '/'), d('var', '/')],
  '/home': [d('u', '/home')],
  '/home/u': [d('projects', '/home/u'), d('.config', '/home/u'), f('notes.txt', '/home/u', 1234), f('big.iso', '/home/u', 5_000_000_000), f('.bashrc', '/home/u', 220), f('Zeta.log', '/home/u', 90), f('alpha.log', '/home/u', 20_000), d('link-to-etc', '/home/u', { isSymlink: true })],
  '/home/u/projects': [f('main.rs', '/home/u/projects', 800), d('src', '/home/u/projects')],
  '/home/u/projects/src': [],
  '/etc': [f('passwd', '/etc', 1900), f('hosts', '/etc', 200), d('ssh', '/etc')],
}
;(window as any).__emitCwd = (path: string) => emit((window as any).__lastEvent, { type: 'cwd', path })
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
  session_test: ({ session }) => {
    if (session.host === 'newhost') return { status: 'unknownHostKey', message: 'Il server risponde, ma la sua host key non è ancora salvata.', elapsedMs: null, keyType: 'ssh-ed25519', fingerprint: 'SHA256:xyzxyz' }
    if (session.host === 'bad') return { status: 'error', message: 'autenticazione rifiutata dal server', elapsedMs: null, keyType: null, fingerprint: null }
    return { status: 'ok', message: 'Connessione e autenticazione riuscite.', elapsedMs: 123, keyType: null, fingerprint: null }
  },
  session_duplicate: ({ id, copySuffix }) => {
    const orig: any = nodes.find((n) => n.id === id)
    const copy = { ...JSON.parse(JSON.stringify(orig)), id: crypto.randomUUID(), name: `${orig.name} ${copySuffix}`, credentialRef: null }
    nodes.push(copy)
    return copy
  },
  ssh_resize: () => {},
  sftp_home: () => '/home/u',
  sftp_list: ({ path }) => {
    if (!(path in fakeFs)) throw `percorso non trovato: ${path}`
    return fakeFs[path]
  },
  ssh_write: () => {},
  ssh_close: () => {},
  ssh_inject_cwd: () => {},
  known_hosts_list: () => [
    { host: '127.0.0.1', port: 2222, keyType: 'ssh-ed25519', fingerprint: 'SHA256:abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG' },
    { host: '10.0.0.1', port: 22, keyType: 'ssh-rsa', fingerprint: 'SHA256:zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz' },
  ],
  known_hosts_remove: () => 1,
  hostkey_reply: ({ decision }) => {
    ;(window as any).__lastDecision = decision
    pendingReply?.(decision)
  },
  app_paths: () => ({ dataDir: 'C:\\Users\\mock\\PiCoXterm', auditLog: 'C:\\Users\\mock\\PiCoXterm\\logs\\audit.log' }),
  sessions_export: () => nodes.length,
  sessions_import: () => 3,
  'plugin:dialog|save': () => 'C:\\tmp\\export.json',
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
  // host "newhost" = chiave sconosciuta; "changed" = chiave cambiata; altro = chiede la password
  ssh_connect: async ({ request, onEvent }) => {
    if (request.host === 'srv') {
      ;(window as any).__lastEvent = onEvent
      return 7
    }
    if (request.host === 'changed') {
      emit(onEvent, { type: 'hostKeyChanged', host: 'changed', port: 22, expected: 'SHA256:vecchia1111', got: 'SHA256:nuova2222' })
      throw 'hostkey-changed'
    }
    if (request.host === 'newhost') {
      emit(onEvent, { type: 'hostKey', reqId: 1, host: 'newhost', port: 22, keyType: 'ssh-ed25519', fingerprint: 'SHA256:xyzxyzxyz' })
      const decision = await new Promise<string>((r) => (pendingReply = r))
      if (decision === 'reject') throw 'host key rifiutata dall’utente'
      return 1
    }
    throw 'password-required'
  },
  'plugin:path|resolve_directory': () => 'C:\\Users\\mock',
  'plugin:path|join': ({ paths }) => paths.join('\\'),
  'plugin:dialog|open': (args) => {
    ;(window as any).__lastOpen = args
    return null
  },
}

;(window as any).__TAURI_INTERNALS__ = {
  invoke: async (cmd: string, args: any) => {
    const h = handlers[cmd]
    if (!h) throw new Error(`mock: comando non gestito ${cmd}`)
    // Come l'IPC reale: gli argomenti passano per JSON
    return h(args === undefined ? args : JSON.parse(JSON.stringify(args)))
  },
  transformCallback: (fn: (m: unknown) => void) => {
    const id = ++nextCallback
    callbacks.set(id, fn)
    return id
  },
  unregisterCallback: (id: number) => callbacks.delete(id),
  convertFileSrc: (p: string) => p,
  metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
}
