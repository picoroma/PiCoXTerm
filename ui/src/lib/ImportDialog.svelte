<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { open } from '@tauri-apps/plugin-dialog'
  import Modal from './Modal.svelte'
  import { t } from '../i18n'

  let { kind, onclose, ondone }: { kind: 'putty' | 'mobaxterm'; onclose: () => void; ondone: (n: number) => void } = $props()

  interface Candidate {
    folder: string[]
    name: string
    protocol: string
    host: string
    port: number
    username: string
    keyPath: string | null
  }
  interface Scan {
    candidates: Candidate[]
    skipped: { name: string; reason: string }[]
  }

  const label = $derived(kind === 'putty' ? 'PuTTY' : 'MobaXterm')
  let path = $state<string | null>(null)
  let scan = $state<Scan | null>(null)
  let chosen = $state<boolean[]>([])
  let error = $state('')
  let busy = $state(false)

  async function load(p: string | null) {
    error = ''
    scan = null
    path = p
    try {
      scan = await invoke<Scan>('import_scan', { source: { kind, path: p } })
      chosen = scan.candidates.map(() => true)
    } catch (e) {
      error = String(e)
    }
  }

  async function choose() {
    const filters =
      kind === 'putty'
        ? [{ name: 'PuTTY (.reg)', extensions: ['reg'] }]
        : [{ name: 'MobaXterm', extensions: ['ini', 'mxtsessions'] }]
    const p = await open({ multiple: false, directory: false, filters })
    if (typeof p === 'string') await load(p)
  }

  async function chooseFolder() {
    const p = await open({ multiple: false, directory: true })
    if (typeof p === 'string') await load(p)
  }

  const count = $derived(chosen.filter(Boolean).length)

  async function run() {
    if (!scan) return
    busy = true
    error = ''
    try {
      const selected = chosen.flatMap((c, i) => (c ? [i] : []))
      const day = new Date().toISOString().slice(0, 10)
      const n = await invoke<number>('import_apply', {
        source: { kind, path },
        selected,
        folderName: `${t('importedFrom', { name: label })} ${day}`,
      })
      ondone(n)
    } catch (e) {
      error = String(e)
      busy = false
    }
  }

  // PuTTY si legge da solo (registro o ~/.putty); MobaXterm chiede il file.
  onMount(() => {
    if (kind === 'putty') void load(null)
  })
</script>

<Modal title={t('importFrom', { name: label })} {onclose} width="680px">
  <p class="hint">{kind === 'putty' ? t('importPuttyHint') : t('importMobaHint')}</p>
  <div class="row">
    <button type="button" onclick={choose}>{t('importChooseFile')}</button>
    {#if kind === 'putty'}
      <button type="button" onclick={chooseFolder}>{t('importChooseFolder')}</button>
    {/if}
  </div>
  {#if error}<p class="error">{error}</p>{/if}
  {#if scan}
    {#if scan.candidates.length === 0}
      <p class="hint">{t('importNone')}</p>
    {:else}
      <div class="head">
        <span>{t('importFound', { n: scan.candidates.length })}</span>
        <button type="button" class="link" onclick={() => (chosen = chosen.map(() => true))}>{t('selectAll')}</button>
        <button type="button" class="link" onclick={() => (chosen = chosen.map(() => false))}>{t('selectNone')}</button>
      </div>
      <div class="list">
        {#each scan.candidates as c, i (i)}
          <label class="item">
            <input type="checkbox" bind:checked={chosen[i]} />
            <span class="name">{[...c.folder, c.name].join(' / ')}</span>
            <span class="dim">{c.protocol.toUpperCase()} · {c.username ? `${c.username}@` : ''}{c.host}:{c.port}{c.keyPath ? ' · 🔑' : ''}</span>
          </label>
        {/each}
      </div>
    {/if}
    {#if scan.skipped.length > 0}
      <details>
        <summary>{t('importSkipped', { n: scan.skipped.length })}</summary>
        <ul>
          {#each scan.skipped as s (s.name)}
            <li>{s.name}: <span class="dim">{s.reason}</span></li>
          {/each}
        </ul>
      </details>
    {/if}
    <p class="hint">{t('importNoPasswords')}</p>
  {/if}
  {#snippet actions()}
    <button type="button" onclick={onclose}>{t('cancel')}</button>
    <button type="button" class="primary" disabled={busy || count === 0} onclick={run}>{t('importDo', { n: count })}</button>
  {/snippet}
</Modal>

<style>
  .row {
    display: flex;
    gap: 8px;
    margin: 8px 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 8px;
  }
  .head span {
    margin-right: auto;
  }
  .link {
    background: none;
    border: 0;
    color: var(--accent);
    padding: 0;
    cursor: pointer;
  }
  .list {
    max-height: 300px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    margin: 6px 0;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
  }
  .item:hover {
    background: var(--hover, rgba(255, 255, 255, 0.06));
  }
  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim,
  .hint {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .error {
    color: var(--danger);
  }
  details {
    margin-top: 6px;
    font-size: 12px;
  }
</style>
