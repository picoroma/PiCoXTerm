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
const ftpTls = { ...newSession(root.id, 'ftp'), name: 'Partner FTPS', host: 'ftp.test', port: 21, username: 'test' }
ftpTls.credentialRef = { provider: 'local', id: 'f1' }
const ftpPlain = { ...newSession(null, 'ftp'), name: 'FTP in chiaro', host: 'ftp.plain', port: 21, username: 'test' }
ftpPlain.credentialRef = { provider: 'local', id: 'f2' }
ftpPlain.ftp = { security: 'plain', pinnedCert: null, startPath: null, showHidden: false }
const s3Test = { ...newSession(root.id, 's3'), name: 'Backup S3', host: 's3.amazonaws.com', port: 443, username: 'pxtaccess' }
s3Test.credentialRef = { provider: 'local', id: 's1' }
s3Test.s3 = { region: 'us-east-1', endpoint: 'http://127.0.0.1:9000', pathStyle: true, bucket: null, profile: null, showHidden: false }
const rdpWin = { ...newSession(root.id, 'rdp'), name: 'Server Windows (RDP)', host: 'win01.example.com', port: 3389, username: 'admin' }
rdpWin.credentialRef = { provider: 'local', id: 'r1' }
const vncBox = { ...newSession(root.id, 'vnc'), name: 'Server VNC (VNC)', host: 'vnc01.example.com', port: 5900, username: '' }
vncBox.credentialRef = { provider: 'local', id: 'v1' }
let nodes: SessionNode[] = [root, pw, key, prod, test, files, filesPw, ftpTls, ftpPlain, s3Test, rdpWin, vncBox]

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
const dataChannels = new Map<number, string>()
;(window as any).__emitData = (text: string, connId?: number) => {
  const ch = connId !== undefined ? dataChannels.get(connId) : [...dataChannels.values()].pop()
  if (ch) emit(ch, Array.from(new TextEncoder().encode(text)))
}
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

// Account S3 finto: bucket, prefissi e oggetti.
const s3Conns = new Set<number>()
const s3d = (name: string, parent: string) => ({ ...d(name, parent), size: 0, modified: null, mode: null, owner: null, group: null, uid: null, gid: null })
const s3f = (name: string, parent: string, size: number) => ({ ...f(name, parent, size), mode: null, owner: null, group: null, uid: null, gid: null })
const day = (n: number) => 1_790_900_000 + (n % 16) * 86_400
const s3t = (name: string, parent: string, size: number, n: number) => ({ ...s3f(name, parent, size), modified: day(n) })
const fakeS3: Record<string, any[]> = {
  '/': [s3d('backups-2026', '/'), s3d('logs-prod', '/'), s3d('media-assets', '/'), s3d('releases', '/')],
  '/backups-2026': [
    s3d('db', '/backups-2026'),
    s3d('config', '/backups-2026'),
    s3t('LEGGIMI.txt', '/backups-2026', 640, 1),
    s3t('hana-full-2026-10-01.tar.gz', '/backups-2026', 4_812_000_000, 5),
    s3t('hana-full-2026-10-08.tar.gz', '/backups-2026', 4_903_000_000, 12),
    s3t('hana-full-2026-10-15.tar.gz', '/backups-2026', 4_955_000_000, 19),
    s3t('inventario.csv', '/backups-2026', 182_400, 20),
  ],
  '/backups-2026/db': [s3t('catalog.bak', '/backups-2026/db', 73_000_000, 4)],
  '/backups-2026/config': [s3t('nginx.conf', '/backups-2026/config', 2_100, 3)],
  '/logs-prod': [s3t('app-2026-10-16.log.gz', '/logs-prod', 18_400_000, 18), s3t('app-2026-10-17.log.gz', '/logs-prod', 21_900_000, 19)],
  '/media-assets': [s3t('logo.png', '/media-assets', 5_400, 2), s3t('banner.jpg', '/media-assets', 1_240_000, 6)],
  '/releases': [s3t('picoxterm-0.1.0-beta.1.zip', '/releases', 11_800_000, 14)],
}
const s3Info: Record<string, any> = {}

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
  sftp_home: ({ id }) => (s3Conns.has(id) ? '/' : '/home/u'),
  // VNC finto: un piccolo server RFB (versione 3.8, nessuna autenticazione) che risponde a noVNC.
  vnc_connect: ({ request, onData, onEvent }) => {
    ;(window as any).__lastVncConnect = request
    const sess: any = nodes.find((n) => n.id === request.sessionId)
    if (!sess.credentialRef && !request.password) throw 'password-required'
    const id = ++connSeq
    const W = 640
    const H = 400
    const out = (bytes: number[]) => emit(onData, bytes)
    const log: any[] = ((window as any).__vncMsgs = [])
    ;(window as any).__vncEnd = (reason: string) => emit(onEvent, { type: 'vncClosed', reason })
    // Finto handshake: versione, un tipo di sicurezza (None), esito OK.
    out([...new TextEncoder().encode('RFB 003.008\n')])
    out([1, 1, 0, 0, 0, 0])
    let buf: number[] = []
    let stage = 'hello'
    let sentFrame = false
    const serverInit = () => {
      const name = [...new TextEncoder().encode('Mock VNC')]
      const b = [W >> 8, W & 255, H >> 8, H & 255, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0, 0, 0, 0, name.length, ...name]
      out(b)
    }
    const update = () => {
      const px: number[] = []
      for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) px.push(80 + Math.round((y / H) * 120), 120, 150 + Math.round((x / W) * 80), 0)
      out([0, 0, 0, 1, 0, 0, 0, 0, W >> 8, W & 255, H >> 8, H & 255, 0, 0, 0, 0, ...px])
    }
    ;(window as any).__vncFeed = (data: number[]) => {
      buf.push(...data)
      for (;;) {
        if (stage === 'hello') {
          if (buf.length < 13) return
          buf = buf.slice(13)
          stage = 'init'
        } else if (stage === 'init') {
          if (buf.length < 1) return
          buf = buf.slice(1)
          serverInit()
          stage = 'run'
        } else {
          if (buf.length < 1) return
          const t = buf[0]
          let n = 0
          if (t === 0) n = 20
          else if (t === 2) {
            if (buf.length < 4) return
            n = 4 + 4 * ((buf[2] << 8) | buf[3])
          } else if (t === 3) n = 10
          else if (t === 4) n = 8
          else if (t === 5) n = 6
          else if (t === 6) {
            if (buf.length < 8) return
            n = 8 + ((buf[4] << 24) | (buf[5] << 16) | (buf[6] << 8) | buf[7])
          } else if (t === 251) n = 8
          else if (t === 255) return
          else n = buf.length
          if (buf.length < n) return
          const msg = buf.slice(0, n)
          buf = buf.slice(n)
          log.push({ type: t, bytes: n, data: t === 4 || t === 5 || t === 6 ? msg : undefined })
          if (t === 3 && !sentFrame) {
            sentFrame = true
            update()
          }
        }
      }
    }
    return id
  },
  vnc_send: (_a: any) => {},
  vnc_close: () => {
    ;(window as any).__vncClosed = true
  },
  rdp_engine_available: () => (window as any).__noEngine !== true,
  // RDP integrato finto: un desktop a gradiente, un riquadro che segue il mouse e la conferma del certificato.
  rdp_connect: async ({ request, onEvent, onFrame }) => {
    ;(window as any).__lastRdpConnect = request
    const sess: any = nodes.find((n) => n.id === request.sessionId)
    const pinned = request.pinnedCert ?? sess.rdp.pinnedCert
    if (!pinned) {
      emit(onEvent, { type: 'certUntrusted', host: sess.host, port: sess.port, fingerprint: 'AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89', changed: false })
      throw 'certificate-untrusted'
    }
    if (!sess.credentialRef && !request.password) throw 'password-required'
    const w = request.width
    const h = request.height
    const id = ++connSeq
    ;(window as any).__rdpInputs = []
    const frame = (x: number, y: number, fw: number, fh: number, color: (px: number, py: number) => [number, number, number]) => {
      const b = new Uint8Array(9 + fw * fh * 4)
      b[0] = 1
      const dv = new DataView(b.buffer)
      dv.setUint16(1, x, true)
      dv.setUint16(3, y, true)
      dv.setUint16(5, fw, true)
      dv.setUint16(7, fh, true)
      for (let j = 0; j < fh; j++) {
        for (let i = 0; i < fw; i++) {
          const [r, g, bl] = color(x + i, y + j)
          const o = 9 + (j * fw + i) * 4
          b[o] = r
          b[o + 1] = g
          b[o + 2] = bl
          b[o + 3] = 255
        }
      }
      return Array.from(b)
    }
    setTimeout(() => {
      emit(onFrame, frame(0, 0, w, h, (px, py) => [Math.round((px / w) * 80), 90 + Math.round((py / h) * 80), 160]))
      emit(onFrame, frame(40, 40, 260, 70, () => [235, 235, 235]))
    }, 50)
    ;(window as any).__rdpRemoteCopy = (text: string) => emit(onFrame, Array.from([3, ...new TextEncoder().encode(text)]))
    // Il "server" conferma una nuova risoluzione e ridisegna tutto con quella dimensione.
    ;(window as any).__rdpResizeReply = (nw: number, nh: number) => {
      const m = new Uint8Array(5)
      m[0] = 4
      new DataView(m.buffer).setUint16(1, nw, true)
      new DataView(m.buffer).setUint16(3, nh, true)
      emit(onFrame, Array.from(m))
      emit(onFrame, frame(0, 0, nw, nh, (px, py) => [200, Math.round((px / nw) * 90), 60 + Math.round((py / nh) * 100)]))
    }
    ;(window as any).__rdpMark = (x: number, y: number) => emit(onFrame, frame(x, y, 24, 24, () => [220, 40, 40]))
    ;(window as any).__rdpEnd = (reason: string) => {
      const enc = new TextEncoder().encode(reason)
      emit(onFrame, Array.from([2, ...enc]))
    }
    return { id, width: w, height: h }
  },
  rdp_resize: ({ width, height }) => {
    ;((window as any).__rdpResizes ??= []).push({ width, height })
    ;(window as any).__rdpResizeReply?.(width, height)
  },
  rdp_clipboard_set: ({ text }) => {
    ;((window as any).__rdpClipSent ??= []).push(text)
  },
  rdp_input: ({ input }) => {
    ;((window as any).__rdpInputs ??= []).push(input)
  },
  window_detach: ({ request }) => {
    const label = `detached-${request.kind}-${request.connId}`
    ;((window as any).__detached ??= []).push({ label, request })
    return label
  },
  window_focus: () => {},
  window_finish: ({ label, action, info }) => {
    ;((window as any).__finished ??= []).push({ label, action, info })
  },
  rdp_attach: async ({ id, onFrame }) => {
    const w = 640
    const h = 400
    ;(window as any).__rdpAttached = id
    setTimeout(() => {
      const b = new Uint8Array(9 + w * h * 4)
      b[0] = 1
      const dv = new DataView(b.buffer)
      dv.setUint16(1, 0, true)
      dv.setUint16(3, 0, true)
      dv.setUint16(5, w, true)
      dv.setUint16(7, h, true)
      for (let i = 0; i < w * h; i++) {
        b[9 + i * 4] = 30
        b[9 + i * 4 + 1] = 160
        b[9 + i * 4 + 2] = 90
        b[9 + i * 4 + 3] = 255
      }
      emit(onFrame, Array.from(b))
    }, 50)
    return { id, width: w, height: h }
  },
  rdp_close: () => {
    ;(window as any).__rdpClosed = true
  },
  rdp_open: ({ sessionId, password }) => {
    ;(window as any).__lastRdp = { sessionId, password }
  },
  s3_connect: ({ request }) => {
    ;(window as any).__lastS3Connect = request
    const s: any = nodes.find((n) => n.id === request.sessionId)
    if (!s.s3.profile && !request.password && !s.credentialRef) throw 'password-required'
    const id = ++connSeq
    s3Conns.add(id)
    return id
  },
  s3_object_info: ({ path }) => {
    const entry = Object.values(fakeS3).flat().find((e: any) => e.path === path)
    return (s3Info[path] ??= { size: entry?.size ?? 120, modified: entry?.modified ?? 1760100000, etag: '9b2cf535f27731c974343645a3985328-3', storageClass: 'STANDARD', contentType: 'text/plain', cacheControl: null, contentDisposition: null, contentEncoding: null, metadata: { progetto: 'picoxterm' }, versionId: null, encryption: null, tags: [['env', 'prod']] })
  },
  s3_set_headers: ({ path, headers }) => {
    ;(window as any).__lastS3Headers = { path, headers }
    Object.assign((s3Info[path] ??= {}), { contentType: headers.contentType, cacheControl: headers.cacheControl, metadata: headers.metadata })
  },
  s3_set_tags: ({ path, tags }) => {
    ;(window as any).__lastS3Tags = { path, tags }
    ;(s3Info[path] ??= {}).tags = tags
  },
  s3_set_storage_class: ({ path, class: c }) => {
    ;(window as any).__lastS3Class = { path, c }
    ;(s3Info[path] ??= {}).storageClass = c
  },
  s3_presign: ({ path, expiresSecs }) => `http://127.0.0.1:9000${path}?X-Amz-Expires=${expiresSecs}&X-Amz-Signature=fake`,
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
  sftp_list: ({ id, path }) => {
    if (s3Conns.has(id)) {
      if (!(path in fakeS3)) throw `non trovato: ${path}`
      return fakeS3[path]
    }
    if (!(path in fakeFs)) throw `percorso non trovato: ${path}`
    return fakeFs[path]
  },
  ssh_write: ({ data }) => {
    ;(window as any).__lastWrite = new TextDecoder().decode(new Uint8Array(data))
  },
  ssh_close: () => {},
  // Il terminale passa a una scheda nuova: il backend rimanda le ultime uscite e prende i nuovi canali.
  ssh_attach: ({ id, onData, onEvent }) => {
    ;(window as any).__sshAttached = id
    eventChannels.set(id, onEvent)
    dataChannels.set(id, onData)
    emit(onData, Array.from(new TextEncoder().encode('replayed-screen\r\n')))
    emit(onEvent, { type: 'cwd', path: '/home/u/projects' })
  },
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
  ssh_connect: async ({ request, onEvent, onData }) => {
    // Le sessioni salvate arrivano con il solo id: l'host si ricava dall'elenco.
    request = { ...request, host: request.host ?? (nodes.find((n) => n.id === request.sessionId) as any)?.host }
    if (request.host === 'srv' || request.host === 'srv-ash') {
      ;(window as any).__lastEvent = onEvent
      const connId = ++connSeq
      eventChannels.set(connId, onEvent)
      dataChannels.set(connId, onData)
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
  'plugin:event|listen': ({ event, handler }) => {
    const w = window as any
    ;(w.__listeners ??= {})[event] = [...(w.__listeners[event] ?? []), handler]
    w.__emitEvent ??= (name: string, payload: unknown) => {
      for (const h of w.__listeners[name] ?? []) callbacks.get(h)?.({ event: name, id: 0, payload })
    }
    return 1
  },
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
    // L'invio dei byte di noVNC passa un Uint8Array, che non sopravvive al giro JSON: va al finto server così com'è.
    if (cmd === 'vnc_send') return (window as any).__vncFeed?.(Array.from(args as Uint8Array))
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
