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
  | { type: 'finished'; jobId: number; summary: { filesDone: number; filesSkipped: number; bytes: number; failed: Failure[] } }
  | { type: 'cancelled'; jobId: number }
  | { type: 'failed'; jobId: number; message: string }

export interface Job {
  id: number
  direction: 'upload' | 'download'
  title: string
  state: 'running' | 'done' | 'cancelled' | 'failed'
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

/** Avvia un piano. `onSettled` viene chiamato a trasferimento concluso, comunque sia finito. */
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
      job.bytesDone = job.bytesTotal
    } else if (ev.type === 'cancelled') {
      job.state = 'cancelled'
    } else {
      job.state = 'failed'
      job.message = ev.message
    }
    job.speedBps = 0
    onSettled?.()
  }
  try {
    job.id = await invoke<number>('transfer_start', { planId: plan.planId, policy, onEvent: channel })
  } catch (e) {
    job.state = 'failed'
    job.message = String(e)
  }
}

export const cancelJob = (job: Job) => invoke('transfer_cancel', { jobId: job.id })

export const discardPlan = (plan: PlanSummary) => invoke('transfer_plan_discard', { planId: plan.planId })

export function dismissJob(job: Job) {
  const i = jobs.indexOf(job)
  if (i >= 0) jobs.splice(i, 1)
}

export function clearFinished() {
  for (let i = jobs.length - 1; i >= 0; i--) if (jobs[i].state !== 'running') jobs.splice(i, 1)
}
