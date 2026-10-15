<script lang="ts">
  import { t } from '../i18n'
  import { cancelJob, clearFinished, dismissJob, jobs, type Job } from './transfers.svelte'

  let collapsed = $state(false)
  const running = $derived(jobs.filter((j) => j.state === 'running').length)

  function size(n: number): string {
    if (n < 1024) return `${n} B`
    const units = ['KiB', 'MiB', 'GiB', 'TiB']
    let v = n
    let u = -1
    do {
      v /= 1024
      u++
    } while (v >= 1024 && u < units.length - 1)
    return `${v.toFixed(v < 10 ? 1 : 0)} ${units[u]}`
  }

  function eta(j: Job): string {
    if (j.speedBps <= 0 || j.bytesTotal <= j.bytesDone) return ''
    const s = Math.round((j.bytesTotal - j.bytesDone) / j.speedBps)
    if (s < 60) return `${s} s`
    if (s < 3600) return `${Math.floor(s / 60)} min ${s % 60} s`
    return `${Math.floor(s / 3600)} h ${Math.floor((s % 3600) / 60)} min`
  }

  const percent = (j: Job) => (j.bytesTotal > 0 ? Math.min(100, (j.bytesDone / j.bytesTotal) * 100) : j.state === 'done' ? 100 : 0)
  const name = (path: string) => path.split(/[\\/]/).pop() ?? path
</script>

{#if jobs.length > 0}
  <section class="bar" aria-label={t('transfers')}>
    <div class="head">
      <strong>{t('transfers')}</strong>
      <span class="dim">{running > 0 ? t('transfersRunning', { n: running }) : t('transfersIdle')}</span>
      <span class="spacer"></span>
      {#if jobs.some((j) => j.state !== 'running')}
        <button type="button" onclick={clearFinished}>{t('clearFinished')}</button>
      {/if}
      <button type="button" aria-label={collapsed ? t('expand') : t('collapse')} title={collapsed ? t('expand') : t('collapse')} onclick={() => (collapsed = !collapsed)}>{collapsed ? '▲' : '▼'}</button>
    </div>
    {#if !collapsed}
      <div class="list">
        {#each jobs as j (j)}
          <div class="job {j.state}">
            <div class="line">
              <span class="dir">{j.direction === 'upload' ? '⬆' : '⬇'}</span>
              <span class="title" title={j.title}>{j.title}</span>
              {#if j.state === 'running'}
                <button type="button" onclick={() => cancelJob(j)}>{t('cancel')}</button>
              {:else}
                <button type="button" aria-label={t('dismiss')} title={t('dismiss')} onclick={() => dismissJob(j)}>✕</button>
              {/if}
            </div>
            <div class="track" role="progressbar" aria-valuenow={Math.round(percent(j))} aria-valuemin="0" aria-valuemax="100">
              <div class="fill" style:width="{percent(j)}%"></div>
            </div>
            <div class="meta">
              {#if j.state === 'running'}
                <span>{size(j.bytesDone)} / {size(j.bytesTotal)}</span>
                <span>{j.filesDone}/{j.filesTotal} {t('files')}</span>
                {#if j.speedBps > 0}<span>{size(j.speedBps)}/s</span>{/if}
                {#if eta(j)}<span>{t('remaining', { time: eta(j) })}</span>{/if}
                <span class="cur" title={j.current}>{name(j.current)}</span>
              {:else if j.state === 'done'}
                <span>{j.failed.length === 0 ? '✓ ' + t('done') : '⚠ ' + t('doneWithErrors', { n: j.failed.length })}</span>
                {#if j.skipped > 0}<span>{t('skipped', { n: j.skipped })}</span>{/if}
              {:else if j.state === 'cancelled'}
                <span>{t('cancelled')}</span>
              {:else}
                <span class="err">{t('failed')}: {j.message}</span>
              {/if}
            </div>
            {#if j.failed.length > 0}
              <ul class="failures">
                {#each j.failed.slice(0, 3) as f (f.path)}
                  <li title={f.message}><code>{name(f.path)}</code>: {f.message}</li>
                {/each}
                {#if j.failed.length > 3}<li>… {t('andMore', { n: j.failed.length - 3 })}</li>{/if}
              </ul>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </section>
{/if}

<style>
  .bar {
    background: var(--bg-elev);
    border-top: 1px solid var(--border);
    max-height: 40%;
    display: flex;
    flex-direction: column;
    flex: none;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
  }
  .head button {
    padding: 1px 8px;
  }
  .spacer {
    flex: 1;
  }
  .dim {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .list {
    overflow: auto;
    padding: 0 8px 6px;
  }
  .job {
    padding: 4px 0;
    border-top: 1px solid var(--border);
    font-size: 12px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .line button {
    padding: 1px 8px;
  }
  .track {
    height: 6px;
    background: var(--bg);
    border-radius: 3px;
    overflow: hidden;
    margin: 4px 0;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.15s linear;
  }
  .job.done .fill {
    background: #5cb85c;
  }
  .job.cancelled .fill,
  .job.failed .fill {
    background: var(--danger);
  }
  .meta {
    display: flex;
    gap: 12px;
    color: var(--fg-dim);
    flex-wrap: wrap;
  }
  .cur {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 220px;
  }
  .err {
    color: var(--danger);
  }
  .failures {
    margin: 4px 0 0;
    padding-left: 18px;
    color: var(--danger);
  }
</style>
