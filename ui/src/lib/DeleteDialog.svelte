<script lang="ts">
  import { onMount } from 'svelte'
  import Modal from './Modal.svelte'
  import { t } from '../i18n'
  import type { SftpEntry } from './types'

  let {
    entries,
    busy,
    failures,
    onconfirm,
    oncancel,
  }: {
    entries: SftpEntry[]
    busy: boolean
    /** Voci che non è stato possibile eliminare (dopo un tentativo). */
    failures: { path: string; message: string }[]
    onconfirm: () => void
    oncancel: () => void
  } = $props()

  const folders = $derived(entries.filter((e) => e.kind === 'dir' && !e.isSymlink).length)
  let cancelBtn: HTMLButtonElement | undefined

  // Il focus parte sul pulsante sicuro: un Invio distratto non elimina nulla.
  onMount(() => cancelBtn?.focus())
</script>

<Modal title={t('deleteTitle')} onclose={oncancel}>
  <p>
    {entries.length === 1 ? t('deleteOne', { name: entries[0].name }) : t('deleteMany', { n: entries.length })}
  </p>
  <ul class="names">
    {#each entries.slice(0, 6) as e (e.path)}
      <li><span aria-hidden="true">{e.kind === 'dir' ? '📁' : e.isSymlink ? '🔗' : '📄'}</span> {e.name}</li>
    {/each}
    {#if entries.length > 6}<li class="dim">… {t('andMore', { n: entries.length - 6 })}</li>{/if}
  </ul>
  {#if folders > 0}
    <p class="warn">{t('deleteFolderWarn')}</p>
  {/if}
  <p class="dim">{t('deleteIrreversible')}</p>
  {#if failures.length > 0}
    <div class="errs" role="alert">
      <strong>{t('deleteFailed', { n: failures.length })}</strong>
      <ul>
        {#each failures.slice(0, 5) as f (f.path)}
          <li><code>{f.path}</code>: {f.message}</li>
        {/each}
      </ul>
    </div>
  {/if}
  {#snippet actions()}
    <button type="button" bind:this={cancelBtn} onclick={oncancel}>{failures.length > 0 ? t('close') : t('cancel')}</button>
    {#if failures.length === 0}
      <button type="button" class="danger" disabled={busy} onclick={onconfirm}>{busy ? t('deleting') : t('delete')}</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .names {
    margin: 0 0 8px;
    padding-left: 18px;
    list-style: none;
    font-size: 13px;
  }
  .warn {
    padding: 8px 10px;
    border-radius: 4px;
    background: color-mix(in srgb, #f0ad4e 22%, transparent);
    border: 1px solid #f0ad4e;
    font-size: 13px;
  }
  .dim {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .errs {
    margin-top: 8px;
    padding: 8px 10px;
    border-radius: 4px;
    border: 1px solid var(--danger);
    font-size: 12px;
    word-break: break-word;
  }
  .errs ul {
    margin: 4px 0 0;
    padding-left: 18px;
  }
</style>
