<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { writeText } from '@tauri-apps/plugin-clipboard-manager'
  import Modal from './Modal.svelte'
  import S3Detail from './S3Detail.svelte'
  import SftpPanel from './SftpPanel.svelte'
  import { t } from '../i18n'
  import type { SftpEntry } from './types'

  let {
    connId,
    startPath = null,
    showHidden = false,
    sessionName = '',
    sessionColor = null,
  }: {
    connId: number
    startPath?: string | null
    showHidden?: boolean
    sessionName?: string
    sessionColor?: string | null
  } = $props()

  const noTracking = { state: 'off' as const, shell: null, message: null }

  let remote = $state<SftpPanel>()
  let local = $state<SftpPanel>()
  let remotePath = $state('')
  let localPath = $state('')
  let selection = $state<SftpEntry[]>([])
  let buckets = $state<string[]>([])
  let bucketsError = $state('')

  // Il pannello del computer è facoltativo e di norma spento: S3 si usa soprattutto da solo.
  const LOCAL_KEY = 'pxt.s3ShowLocal'
  function loadShowLocal(): boolean {
    try {
      return localStorage.getItem(LOCAL_KEY) === '1'
    } catch {
      return false
    }
  }
  let showLocal = $state(loadShowLocal())
  function toggleLocal() {
    showLocal = !showLocal
    try {
      localStorage.setItem(LOCAL_KEY, showLocal ? '1' : '0')
    } catch {
      /* è solo una comodità */
    }
  }

  const currentBucket = $derived(remotePath.split('/').filter(Boolean)[0] ?? '')

  async function loadBuckets() {
    try {
      const list = await invoke<SftpEntry[]>('sftp_list', { id: connId, path: '/' })
      buckets = list.filter((e) => e.kind === 'dir').map((e) => e.name)
      bucketsError = ''
    } catch (e) {
      bucketsError = String(e)
    }
  }
  onMount(() => void loadBuckets())

  /** Un trascinamento è finito: se è sull'altro pannello, si trasferisce. */
  function dropped(source: 'local' | 'remote', paths: string[], x: number, y: number) {
    const target = document.elementFromPoint(x, y)?.closest('[data-pane]')?.getAttribute('data-pane')
    if (!target || target === source) return
    if (source === 'local') remote?.uploadPaths(paths)
    else if (localPath) void remote?.downloadPaths(paths, localPath)
  }

  // ---- Link temporaneo ----
  let presign = $state<{ entry: SftpEntry; secs: number; url: string; error: string; busy: boolean; copied: boolean } | null>(null)
  const DURATIONS = [
    { secs: 15 * 60, label: () => t('s3PresignMinutes', { n: 15 }) },
    { secs: 3600, label: () => t('s3PresignHours', { n: 1 }) },
    { secs: 12 * 3600, label: () => t('s3PresignHours', { n: 12 }) },
    { secs: 24 * 3600, label: () => t('s3PresignDays', { n: 1 }) },
    { secs: 7 * 24 * 3600, label: () => t('s3PresignDays', { n: 7 }) },
  ]

  async function createPresign() {
    if (!presign) return
    presign.busy = true
    presign.error = ''
    try {
      presign.url = await invoke<string>('s3_presign', { id: connId, path: presign.entry.path, expiresSecs: presign.secs })
    } catch (e) {
      presign.error = String(e)
    } finally {
      presign.busy = false
    }
  }

  async function copyUrl() {
    if (!presign?.url) return
    await writeText(presign.url)
    presign.copied = true
  }
</script>

<div class="s3" style:grid-template-columns="150px {showLocal ? '1fr 1fr' : '1fr'} 320px">
  <nav class="buckets" aria-label={t('s3Buckets')}>
    <div class="head">
      <span>{t('s3Buckets')}</span>
      <button type="button" class:on={showLocal} aria-pressed={showLocal} title={t('s3ShowLocal')} onclick={toggleLocal}>💻</button>
    </div>
    {#if bucketsError}<p class="err">{bucketsError}</p>{/if}
    {#each buckets as b (b)}
      <button type="button" class="bucket" class:cur={b === currentBucket} onclick={() => remote?.goto(`/${b}`)}>🪣 {b}</button>
    {/each}
    <button type="button" class="bucket" class:cur={currentBucket === ''} onclick={() => remote?.goto('/')}>⌂ /</button>
  </nav>

  {#if showLocal}
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
  {/if}
  <div class="side">
    <SftpPanel
      bind:this={remote}
      standalone
      s3
      {connId}
      cwd=""
      liveIds={[connId]}
      tracking={noTracking}
      onenabletracking={() => {}}
      {startPath}
      showHiddenInitial={showHidden}
      {sessionName}
      {sessionColor}
      localDir={showLocal ? localPath : undefined}
      onlocalchanged={() => local?.refresh()}
      ondragdrop={(paths, x, y) => dropped('remote', paths, x, y)}
      bind:path={remotePath}
      onselect={(e) => (selection = e)}
      onpresign={(entry) => (presign = { entry, secs: 3600, url: '', error: '', busy: false, copied: false })}
    />
  </div>
  <S3Detail {connId} {selection} onchanged={() => remote?.refresh()} />
</div>

{#if presign}
  <Modal title={t('s3PresignTitle')} onclose={() => (presign = null)}>
    <p class="name">{presign.entry.name}</p>
    <label class="field">
      {t('s3PresignExpires')}
      <select bind:value={presign.secs} disabled={presign.busy} onchange={() => presign && ((presign.url = ''), (presign.copied = false))}>
        {#each DURATIONS as d (d.secs)}<option value={d.secs}>{d.label()}</option>{/each}
      </select>
    </label>
    <p class="warn">{t('s3PresignWarn')}</p>
    {#if presign.url}
      <textarea readonly rows="5" value={presign.url} onfocus={(e) => e.currentTarget.select()}></textarea>
    {/if}
    {#if presign.error}<p class="err">{presign.error}</p>{/if}
    {#snippet actions()}
      <button type="button" onclick={() => (presign = null)}>{t('close')}</button>
      {#if presign?.url}
        <button type="button" class="primary" onclick={copyUrl}>{presign.copied ? t('s3PresignCopied') : t('s3PresignCopy')}</button>
      {:else}
        <button type="button" class="primary" disabled={presign?.busy} onclick={createPresign}>{t('s3PresignCreate')}</button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .s3 {
    display: grid;
    height: 100%;
    min-height: 0;
  }
  .side {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    border-left: 1px solid var(--border);
  }
  .buckets {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: auto;
    background: var(--bg-elev);
    font-size: 13px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px;
    color: var(--fg-dim);
  }
  .head button.on {
    outline: 1px solid var(--accent);
  }
  .bucket {
    text-align: left;
    border: 0;
    border-radius: 0;
    background: none;
    padding: 5px 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bucket:hover {
    background: var(--hover, rgba(255, 255, 255, 0.06));
  }
  .bucket.cur {
    background: var(--accent);
    color: #fff;
  }
  .err {
    color: #ffb3b3;
    margin: 4px 8px;
    font-size: 12px;
    word-break: break-word;
  }
  .name {
    margin: 0 0 8px;
    word-break: break-all;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
    color: var(--fg-dim);
  }
  .warn {
    font-size: 12px;
    color: var(--fg-dim);
  }
  textarea {
    width: 100%;
    font-family: Consolas, monospace;
    font-size: 12px;
    box-sizing: border-box;
  }
</style>
