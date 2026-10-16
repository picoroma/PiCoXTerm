import { Channel, invoke } from '@tauri-apps/api/core'

export type ConflictPolicy = 'overwrite' | 'skip' | 'rename' | 'overwriteIfNewer'

/** Piano di trasferimento calcolato dal backend, in attesa di avvio. */
export interface PlanSummary {
  planId: number
  direction: 'upload' | 'download'
  fileCount: number
  dirCount: number
  totalBytes: number
  conflicts: number
  conflictSamples: string[]
}

interface Failure {
  path: string
  message: string
}

type TransferEvent =
  | {
      type: 'progress'
      jobId: number
      bytesDone: number
      bytesTotal: number
      filesDone: number
      filesTotal: number
      current: string
      speedBps: number
    }
  | {
      type: 'finished'
      jobId: number
      summary: { filesDone: number; filesSkipped: number; bytes: number; failed: Failure[] }
      resumable: boolean
    }
  | { type: 'paused'; jobId: number }
  | { type: 'cancelled'; jobId: number }
  | { type: 'failed'; jobId: number; message: string }

export interface Job {
  id: number
  direction: 'upload' | 'download'
  title: string
  state: 'running' | 'paused' | 'done' | 'cancelled' | 'failed'
  /** Si può riprendere (in pausa) o riprovare sui file falliti: il backend tiene il lavoro. */
  resumable: boolean
  bytesDone: number
  bytesTotal: number
  filesDone: number
  filesTotal: number
  current: string
  speedBps: number
  skipped: number
  failed: Failure[]
  message: string
}

/** Trasferimenti di questa sessione dell'app (in corso e conclusi, finché non vengono tolti dall'elenco). */
export const jobs: Job[] = $state([])

/** Cosa fare (ad esempio aggiornare l'elenco dei file) quando un lavoro si ferma, comunque sia andata. */
const settledHooks = new WeakMap<Job, () => void>()

/** Collega il canale degli eventi del backend a un lavoro dell'elenco. */
function attach(job: Job, channel: Channel<TransferEvent>) {
  channel.onmessage = (ev) => {
    job.id = ev.jobId
    if (ev.type === 'progress') {
      job.bytesDone = ev.bytesDone
      job.bytesTotal = ev.bytesTotal
      job.filesDone = ev.filesDone
      job.filesTotal = ev.filesTotal
      job.current = ev.current
      job.speedBps = ev.speedBps
      return
    }
    if (ev.type === 'finished') {
      job.state = 'done'
      job.skipped = ev.summary.filesSkipped
      job.failed = ev.summary.failed
      job.resumable = ev.resumable
      if (job.failed.length === 0) job.bytesDone = job.bytesTotal
    } else if (ev.type === 'paused') {
      job.state = 'paused'
      job.resumable = true
    } else if (ev.type === 'cancelled') {
      job.state = 'cancelled'
      job.resumable = false
    } else {
      job.state = 'failed'
      job.message = ev.message
    }
    job.speedBps = 0
    settledHooks.get(job)?.()
  }
}

/** Avvia un piano. `onSettled` viene chiamato quando il lavoro si ferma (concluso, in pausa o annullato). */
export async function startTransfer(
  plan: PlanSummary,
  policy: ConflictPolicy,
  title: string,
  onSettled?: () => void,
): Promise<void> {
  const channel = new Channel<TransferEvent>()
  const job = jobs[
    jobs.push({
      id: 0,
      direction: plan.direction,
      title,
      state: 'running',
      resumable: false,
      bytesDone: 0,
      bytesTotal: plan.totalBytes,
      filesDone: 0,
      filesTotal: plan.fileCount,
      current: '',
      speedBps: 0,
      skipped: 0,
      failed: [],
      message: '',
    }) - 1
  ]
  if (onSettled) settledHooks.set(job, onSettled)
  attach(job, channel)
  try {
    job.id = await invoke<number>('transfer_start', { planId: plan.planId, policy, onEvent: channel })
  } catch (e) {
    job.state = 'failed'
    job.message = String(e)
  }
}

/** Riprende un lavoro in pausa o riprova i file falliti: i file già finiti non si rifanno. */
export async function resumeJob(job: Job): Promise<void> {
  if (!job.resumable) return
  const channel = new Channel<TransferEvent>()
  const previous = { state: job.state, failed: job.failed, skipped: job.skipped }
  job.state = 'running'
  job.failed = []
  job.message = ''
  attach(job, channel)
  try {
    job.id = await invoke<number>('transfer_resume', { jobId: job.id, onEvent: channel })
    job.resumable = false
  } catch (e) {
    // Per esempio la connessione è stata chiusa: il lavoro resta com'era, con il motivo.
    job.state = previous.state
    job.failed = previous.failed
    job.skipped = previous.skipped
    job.message = String(e)
  }
}

export const cancelJob = (job: Job) => invoke('transfer_cancel', { jobId: job.id })

export const pauseJob = (job: Job) => invoke('transfer_pause', { jobId: job.id })

/** Abbandona un lavoro riprendibile ed elimina i file parziali che ha lasciato. */
const discardJob = (job: Job) => (job.resumable ? invoke('transfer_discard', { jobId: job.id }).catch(() => {}) : undefined)

export const discardPlan = (plan: PlanSummary) => invoke('transfer_plan_discard', { planId: plan.planId })

/** Toglie un lavoro dall'elenco. Se era in pausa, i suoi file parziali vengono eliminati. */
export function dismissJob(job: Job) {
  const i = jobs.indexOf(job)
  if (i >= 0) {
    void discardJob(job)
    jobs.splice(i, 1)
  }
}

/** Toglie tutti i lavori fermi (conclusi, annullati, in pausa), eliminando i parziali dei sospesi. */
export function clearFinished() {
  for (let i = jobs.length - 1; i >= 0; i--) {
    if (jobs[i].state !== 'running') {
      void discardJob(jobs[i])
      jobs.splice(i, 1)
    }
  }
}
