<script lang="ts">
  import SftpPanel from './SftpPanel.svelte'
  import { t } from '../i18n'
  import type { SftpEntry } from './types'

  let {
    connId,
    startPath = null,
    showHidden = false,
    sessionName = '',
    sessionColor = null,
    onedit,
    onexternal,
  }: {
    connId: number
    startPath?: string | null
    showHidden?: boolean
    sessionName?: string
    sessionColor?: string | null
    onedit?: (connId: number, entry: SftpEntry) => void
    onexternal?: (connId: number, entry: SftpEntry) => void
  } = $props()

  const noTracking = { state: 'off' as const, shell: null, message: null }

  let local = $state<SftpPanel>()
  let remote = $state<SftpPanel>()
  let localPath = $state('')
  let wrap = $state<HTMLDivElement>()

  // Quota di larghezza del pannello locale, ricordata tra un avvio e l'altro.
  const RATIO_KEY = 'pxt.dualRatio'
  function loadRatio(): number {
    try {
      const v = Number(localStorage.getItem(RATIO_KEY))
      return Number.isFinite(v) && v >= 0.2 && v <= 0.8 ? v : 0.5
    } catch {
      return 0.5
    }
  }
  let ratio = $state(loadRatio())

  function dragSplit(e: PointerEvent) {
    e.preventDefault()
    const move = (m: PointerEvent) => {
      if (!wrap) return
      const r = wrap.getBoundingClientRect()
      ratio = Math.min(0.8, Math.max(0.2, (m.clientX - r.left) / r.width))
    }
    const up = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      try {
        localStorage.setItem(RATIO_KEY, String(ratio))
      } catch {
        /* è solo una comodità */
      }
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }

  /** Un trascinamento è finito: se è sull'altro pannello, si trasferisce. */
  function dropped(source: 'local' | 'remote', paths: string[], x: number, y: number) {
    const target = document.elementFromPoint(x, y)?.closest('[data-pane]')?.getAttribute('data-pane')
    if (!target || target === source) return
    if (source === 'local') remote?.uploadPaths(paths)
    else if (localPath) void remote?.downloadPaths(paths, localPath)
  }
</script>

<div class="dual" bind:this={wrap} style:grid-template-columns="{ratio}fr 6px {1 - ratio}fr">
  <div class="side">
    <SftpPanel
      bind:this={local}
      kind="local"
      standalone
      connId={0}
      cwd=""
      liveIds={[0]}
      tracking={noTracking}
      onenabletracking={() => {}}
      showHiddenInitial={showHidden}
      sessionName={t('thisComputer')}
      bind:path={localPath}
      onsend={(paths) => remote?.uploadPaths(paths)}
      ondragdrop={(paths, x, y) => dropped('local', paths, x, y)}
    />
  </div>
  <div class="split" role="separator" aria-orientation="vertical" aria-label={t('resize')} onpointerdown={dragSplit}></div>
  <div class="side">
    <SftpPanel
      bind:this={remote}
      standalone
      {connId}
      cwd=""
      liveIds={[connId]}
      tracking={noTracking}
      onenabletracking={() => {}}
      {startPath}
      showHiddenInitial={showHidden}
      {sessionName}
      {sessionColor}
      {onedit}
      {onexternal}
      localDir={localPath}
      onlocalchanged={() => local?.refresh()}
      ondragdrop={(paths, x, y) => dropped('remote', paths, x, y)}
    />
  </div>
</div>

<style>
  .dual {
    display: grid;
    height: 100%;
    min-height: 0;
  }
  .side {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .split {
    cursor: col-resize;
    background: var(--border);
  }
  .split:hover {
    background: var(--accent);
  }
</style>
