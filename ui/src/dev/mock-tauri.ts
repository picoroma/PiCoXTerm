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
const prod = { ...newSession(null), name: 'PROD srv', host: 'srv', username: 'u' }
prod.ui.color = '#c0392b'
const test = { ...newSession(null), name: 'TEST srv', host: 'srv', username: 'u' }
test.ui.color = '#2f6fed'
const files = { ...newSession(root.id, 'sftp'), name: 'FILES srv (SFTP)', host: 'srv', username: 'u' }
files.sftp = { startPath: '/home/u/projects', showHidden: false }
files.ui.color = '#2e8b57'
const filesPw = { ...newSession(null, 'sftp'), name: 'FILES password (SFTP)', host: '10.0.0.9', username: 'u' }
const ftpTls = { ...newSession(root.id, 'ftp'), name: 'FTPS prova (FTP)', host: 'ftp.test', port: 21, username: 'test' }
ftpTls.credentialRef = { provider: 'local', id: 'f1' }
const ftpPlain = { ...newSession(null, 'ftp'), name: 'FTP in chiaro', host: 'ftp.plain', port: 21, username: 'test' }
ftpPlain.credentialRef = { provider: 'local', id: 'f2' }
ftpPlain.ftp = { security: 'plain', pinnedCert: null, startPath: null, showHidden: false }
let nodes: SessionNode[] = [root, pw, key, prod, test, files, filesPw, ftpTls, ftpPlain]

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
  // Qui il server comunica solo gli id (come OpenSSH): i nomi si risolvono con `sftp_id_names`.
  '/home/u/projects': [f('main.rs', '/home/u/projects', 800, { owner: null, group: null, uid: 1001, gid: 79 }), d('src', '/home/u/projects', { owner: null, group: null, uid: 4242, gid: 4242 })],
  '/home/u/projects/src': [],
  '/conflict': [f('a.txt', '/conflict', 10), f('b.log', '/conflict', 20)],
  '/etc': [f('passwd', '/etc', 1900), f('hosts', '/etc', 200), d('ssh', '/etc')],
}
;// Ogni connessione ha il suo canale di eventi: __emitCwd(path, connId) simula il terminale di quella scheda.
const eventChannels = new Map<number, string>()
let connSeq = 6
;(window as any).__emitCwd = (path: string, connId?: number) =>
  emit(connId !== undefined ? eventChannels.get(connId)! : (window as any).__lastEvent, { type: 'cwd', path })
const le = (name: string, dir: string, kind: 'file' | 'dir', size = 0): any => ({ name, path: `${dir.replace(/\\$/, '')}\\${name}`, kind, isSymlink: false, size, modified: 1_700_000_000, mode: null, owner: null, group: null, uid: null, gid: null })
const fakeLocal: Record<string, any[]> = {
  '': [le('C:', '', 'dir'), le('D:', '', 'dir')].map((e) => ({ ...e, path: `${e.name}\\` })),
  'C:\\': [le('Users', 'C:\\', 'dir'), le('Windows', 'C:\\', 'dir')],
  'C:\\Users': [le('mock', 'C:\\Users', 'dir')],
  'C:\\Users\\mock': [le('Documents', 'C:\\Users\\mock', 'dir')],
  'C:\\Users\\mock\\Documents': [le('relazione.docx', 'C:\\Users\\mock\\Documents', 'file', 48_000), le('foto', 'C:\\Users\\mock\\Documents', 'dir'), le('dati.csv', 'C:\\Users\\mock\\Documents', 'file', 1200), le('script.sh', 'C:\\Users\\mock\\Documents', 'file', 90)],
  'C:\\Users\\mock\\Documents\\foto': [le('mare.jpg', 'C:\\Users\\mock\\Documents\\foto', 'file', 2_000_000)],
}
let externalSeq = 700
const externalChannels = new Map<number, string>()
;(window as any).__externalChanged = (editId: number) => emit(externalChannels.get(editId)!, { type: 'changed', editId })
const fileTexts: Record<string, { content: string; modified: number; size: number }> = {}
;(window as any).__externalEdit = (path: string) => {
  const c = fileTexts[path]
  if (c) {
    c.content += '# modificato da un altro\n'
    c.size += 25
    c.modified += 100
  }
}
const plans = new Map<number, any>()
const jobStates = new Map<number, { mode: 'running' | 'paused' | 'cancelled' }>()
const resumable = new Map<number, { plan: any; policy: string; step: number }>()
let planSeq = 100
let jobSeq = 500
function makePlan(direction: string, fileCount: number, dest: string) {
  const conflicts = dest.includes('conflict') ? 2 : 0
  const plan = { planId: ++planSeq, direction, fileCount, dirCount: 1, totalBytes: fileCount * 10_000_000, conflicts, conflictSamples: conflicts ? ['a.txt', 'b.log'] : [] }
  plans.set(plan.planId, plan)
  return plan
}
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
/** Trasferimento finto: sei passi, con pausa, ripresa e (se `__failTransfer`) un file fallito. */
function runMockJob(rec: { plan: any; policy: string; step: number }, onEvent: string): number {
  const jobId = ++jobSeq
  const state = { mode: 'running' as 'running' | 'paused' | 'cancelled' }
  jobStates.set(jobId, state)
  const total = rec.plan?.totalBytes ?? 1000
  const tick = () => {
    if (state.mode === 'cancelled') return emit(onEvent, { type: 'cancelled', jobId })
    if (state.mode === 'paused') {
      resumable.set(jobId, rec)
      return emit(onEvent, { type: 'paused', jobId })
    }
    rec.step++
    if (rec.step > 5) {
      const failed = (window as any).__failTransfer ? [{ path: '/x/falla.txt', message: 'permesso negato' }] : []
      if (failed.length) resumable.set(jobId, rec)
      return emit(onEvent, { type: 'finished', jobId, resumable: failed.length > 0, summary: { filesDone: rec.plan?.fileCount ?? 0, filesSkipped: rec.policy === 'skip' ? rec.plan?.conflicts ?? 0 : 0, bytes: total, failed } })
    }
    emit(onEvent, { type: 'progress', jobId, bytesDone: Math.round((total * rec.step) / 6), bytesTotal: total, filesDone: rec.step, filesTotal: rec.plan?.fileCount ?? 0, current: `/tmp/file-${rec.step}.bin`, speedBps: 5_000_000 })
    setTimeout(tick, 400)
  }
  setTimeout(tick, 50)
  return jobId
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
  ssh_enable_cwd_tracking: () => ({ type: 'tracking', state: 'active', shell: '/bin/bash', message: null }),
  sftp_home: () => '/home/u',
  // Operazioni di modifica sul filesystem finto
  sftp_delete: ({ paths }) => {
    const failed: any[] = []
    let done = 0
    for (const p of paths) {
      if ((window as any).__failDelete?.includes(p)) {
        failed.push({ path: p, message: 'permesso negato' })
        continue
      }
      const parent = p.slice(0, p.lastIndexOf('/')) || '/'
      fakeFs[parent] = (fakeFs[parent] ?? []).filter((e) => e.path !== p)
      for (const k of Object.keys(fakeFs)) if (k === p || k.startsWith(p + '/')) delete fakeFs[k]
      done++
    }
    ;(window as any).__lastDelete = paths
    return { done, failed }
  },
  sftp_tree_info: () => ({ files: 12, dirs: 4, bytes: 3_456_789, partial: false }),
  sftp_chmod: ({ path, mode, recursive }) => {
    ;(window as any).__lastChmod = { path, mode: mode.toString(8), recursive }
    const parent = path.slice(0, path.lastIndexOf('/')) || '/'
    const e = (fakeFs[parent] ?? []).find((x) => x.path === path)
    if (e) e.mode = mode
    return { done: recursive ? 5 : 1, failed: [] }
  },
  sftp_rename: ({ path, newName }) => {
    const parent = path.slice(0, path.lastIndexOf('/')) || '/'
    const list = fakeFs[parent] ?? []
    if (list.some((e) => e.name === newName)) throw `esiste già una voce chiamata "${newName}"`
    const e = list.find((x) => x.path === path)
    if (e) {
      e.name = newName
      e.path = `${parent === '/' ? '' : parent}/${newName}`
    }
    return e?.path
  },
  sftp_mkdir: ({ parent, name }) => {
    const list = fakeFs[parent] ?? (fakeFs[parent] = [])
    if (list.some((e) => e.name === name)) throw `esiste già una voce chiamata "${name}"`
    list.push(d(name, parent))
    fakeFs[`${parent === '/' ? '' : parent}/${name}`] = []
    return `${parent}/${name}`
  },
  // Editor integrato: contenuti finti in memoria. `__externalEdit(path)` simula la modifica di un altro utente.
  // Editor di sistema finto: `__externalChanged(editId)` simula il salvataggio del programma esterno.
  external_edit_open: ({ path, onEvent }) => {
    if (/\.(sh|py|bat)$/i.test(path)) throw 'Questo tipo di file potrebbe essere eseguito dal sistema invece di aprirsi in un editor: usa l\u2019editor integrato (Modifica).'
    const editId = ++externalSeq
    externalChannels.set(editId, onEvent)
    ;(window as any).__lastExternalOpen = path
    return { editId, name: path.split('/').pop(), reused: false }
  },
  external_edit_push: ({ editId, force }) => {
    ;(window as any).__lastExternalPush = { editId, force }
    if (!force && (window as any).__externalConflict) return { status: 'conflict', size: 10, modified: 1, missing: false }
    ;(window as any).__externalConflict = false
    return { status: 'saved', size: 10, modified: 2 }
  },
  external_edit_stop: ({ editId }) => {
    ;(window as any).__lastExternalStop = editId
    externalChannels.delete(editId)
  },
  sftp_read_text: ({ path }) => {
    if (path.endsWith('.iso')) throw 'file troppo grande per l’editor (5000000000 byte, limite 5242880)'
    const c = (fileTexts[path] ??= { content: `# ${path}\nexport PATH=$PATH:/opt/bin\necho "ciao"\n`, modified: 1_700_000_000, size: 60 })
    return { content: c.content, encoding: path.endsWith('.log') ? 'latin1' : 'utf8', eol: path.endsWith('.log') ? 'crlf' : 'lf', mixedEol: path.endsWith('alpha.log'), size: c.size, modified: c.modified }
  },
  sftp_write_text: ({ path, content, base }) => {
    const c = (fileTexts[path] ??= { content: '', modified: 1_700_000_000, size: 0 })
    if (base && (base.size !== c.size || base.modified !== c.modified)) return { status: 'conflict', size: c.size, modified: c.modified, missing: false }
    c.content = content
    c.size = content.length
    c.modified += 5
    ;(window as any).__lastSaved = { path, content }
    return { status: 'saved', size: c.size, modified: c.modified }
  },
  sftp_id_names: () => ({ users: { '0': 'root', '1001': 'sapadm' }, groups: { '0': 'root', '79': 'sapsys' } }),
  // Disco locale finto (pannello locale del doppio pannello).
  local_home: () => 'C:\\Users\\mock\\Documents',
  local_list: ({ path }) => {
    if (!(path in fakeLocal)) throw `percorso non trovato: ${path}`
    return fakeLocal[path]
  },
  local_open: ({ path }) => {
    ;(window as any).__lastLocalOpen = path
  },
  local_delete: ({ paths }) => {
    ;(window as any).__lastLocalDelete = paths
    return { done: paths.length, failed: [] }
  },
  local_tree_info: () => ({ files: 1, dirs: 1, bytes: 10, partial: false }),
  local_mkdir: ({ parent, name }) => `${parent}\\${name}`,
  local_rename: ({ path, newName }) => path.replace(/[^\\]+$/, newName),
  sftp_list: ({ path }) => {
    if (!(path in fakeFs)) throw `percorso non trovato: ${path}`
    return fakeFs[path]
  },
  ssh_write: ({ data }) => {
    ;(window as any).__lastWrite = new TextDecoder().decode(new Uint8Array(data))
  },
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
  about_info: () => ({ name: 'PiCoXterm', version: '0.1.0', commit: '5c3a9f1', buildDate: '2026-10-16', tauriVersion: '2.12.4', webviewVersion: '141.0.3537.71', os: 'windows', arch: 'x86_64', portable: false, dataDir: 'C:\\Users\\mock\\AppData\\Roaming\\PiCoXterm', auditLog: 'C:\\Users\\mock\\PiCoXterm\\logs\\audit.log' }),
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
    // Le sessioni salvate arrivano con il solo id: l'host si ricava dall'elenco.
    request = { ...request, host: request.host ?? (nodes.find((n) => n.id === request.sessionId) as any)?.host }
    if (request.host === 'srv' || request.host === 'srv-ash') {
      ;(window as any).__lastEvent = onEvent
      const connId = ++connSeq
      eventChannels.set(connId, onEvent)
      const unsupported = request.host === 'srv-ash'
      setTimeout(() => emit(onEvent, unsupported ? { type: 'tracking', state: 'unsupported', shell: '/bin/ash', message: null } : { type: 'tracking', state: 'active', shell: '/bin/bash', message: null }), 400)
      return connId
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
  // Policy dell'organizzazione finta: `window.__allowPlain = false` vieta l'FTP in chiaro.
  policy_get: () => ({ policy: { allowPlainFtp: (window as any).__allowPlain ?? true, allowedProtocols: null }, warning: null }),
  // Sessione FTP finta: FTPS con certificato da confermare (impronta finta), FTP in chiaro se la policy lo permette.
  ftp_connect: async ({ request, onEvent }) => {
    ;(window as any).__lastFtpConnect = request
    const s: any = nodes.find((n) => n.id === request.sessionId)
    if (s.ftp.security === 'plain') {
      if ((window as any).__allowPlain === false) throw 'L’FTP in chiaro è vietato dalla policy dell’organizzazione: usa FTPS (TLS esplicito o implicito).'
      return ++connSeq
    }
    const pinned = request.pinnedCert ?? s.ftp.pinnedCert
    if (!pinned) {
      emit(onEvent, { type: 'certUntrusted', host: s.host, port: s.port, fingerprint: 'AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89', changed: false })
      throw 'certificate-untrusted'
    }
    if (pinned === 'CAMBIATO') {
      emit(onEvent, { type: 'certUntrusted', host: s.host, port: s.port, fingerprint: 'FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF:FF', changed: true })
      throw 'certificate-untrusted'
    }
    return ++connSeq
  },
  // Sessione SFTP finta: stessa logica di ssh_connect, senza terminale.
  sftp_connect: async ({ request, onEvent }) => {
    request = { ...request, host: request.host ?? (nodes.find((n) => n.id === request.sessionId) as any)?.host }
    ;(window as any).__lastSftpConnect = request
    if (request.host === 'srv') {
      const connId = ++connSeq
      eventChannels.set(connId, onEvent)
      return connId
    }
    if (request.host === '10.0.0.9' && request.password === 'segreta') return ++connSeq
    if (request.host === '10.0.0.9') throw 'password-required'
    throw 'impossibile collegarsi'
  },
  'plugin:path|resolve_directory': () => 'C:\\Users\\mock',
  'plugin:path|join': ({ paths }) => paths.join('\\'),
  // Il test imposta window.__nextOpen con ciò che "sceglie" l'utente nel selettore di file.
  'plugin:dialog|open': (args) => {
    ;(window as any).__lastOpen = args
    const v = (window as any).__nextOpen ?? null
    ;(window as any).__nextOpen = null
    return v
  },
  'plugin:clipboard-manager|write_text': (a) => {
    ;(window as any).__clipboard = a?.text
  },
  'plugin:clipboard-manager|read_text': () => (window as any).__clipboard ?? '',
  'plugin:event|listen': () => 1,
  'plugin:event|unlisten': () => {},
  // Trasferimenti finti: il piano ha conflitti se la cartella di destinazione contiene "conflict".
  transfer_plan_upload: ({ localPaths, remoteDir }) => makePlan('upload', localPaths.length * 2, remoteDir),
  transfer_plan_download: ({ remotePaths, localDir }) => makePlan('download', remotePaths.length * 3, localDir),
  transfer_plan_discard: () => {},
  transfer_start: ({ planId, policy, onEvent }) => {
    ;(window as any).__lastPolicy = policy
    const plan = plans.get(planId)
    return runMockJob({ plan, policy, step: 0 }, onEvent)
  },
  transfer_resume: ({ jobId, onEvent }) => {
    const rec = resumable.get(jobId)
    if (!rec) throw 'lavoro non riprendibile (già ripreso o scartato)'
    resumable.delete(jobId)
    ;(window as any).__failTransfer = false
    return runMockJob({ ...rec, step: rec.step }, onEvent)
  },
  transfer_pause: ({ jobId }) => {
    const st = jobStates.get(jobId)
    if (st) st.mode = 'paused'
  },
  transfer_cancel: ({ jobId }) => {
    const st = jobStates.get(jobId)
    if (st) st.mode = 'cancelled'
  },
  transfer_discard: ({ jobId }) => {
    ;(window as any).__lastDiscard = jobId
    return resumable.delete(jobId) ? 1 : 0
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
