<script lang="ts">
  import { t } from '../i18n'
  import { externals, pushExternal, stopExternal } from './external.svelte'
</script>

{#if externals.length > 0}
  <section class="bar" aria-label={t('externalTitle')}>
    <strong>{t('externalTitle')}</strong>
    {#each externals as e (e.editId)}
      <div class="item {e.state}">
        <span class="name" title={e.path}>✎ {e.name}</span>
        <span class="status" role="status">
          {#if e.state === 'open'}{t('externalOpen')}
          {:else if e.state === 'changed'}{t('externalChanged')}
          {:else if e.state === 'uploading'}{t('externalUploading')}
          {:else if e.state === 'uploaded'}✓ {t('externalUploaded')}
          {:else if e.state === 'conflict'}⚠ {t('editorConflictBody')}
          {:else if e.state === 'missing'}⚠ {t('editorMissingBody')}
          {:else}⚠ {e.message}{/if}
        </span>
        <span class="spacer"></span>
        {#if e.state === 'changed' || e.state === 'open' || e.state === 'uploaded'}
          <label class="check" title={t('externalAutoHint')}>
            <input type="checkbox" bind:checked={e.auto} /> {t('externalAuto')}
          </label>
        {/if}
        {#if e.state === 'changed'}
          <button type="button" class="primary" onclick={() => pushExternal(e, false)}>{t('externalUpload')}</button>
          <button type="button" onclick={() => stopExternal(e)} title={t('externalDiscardHint')}>{t('externalDiscard')}</button>
        {:else if e.state === 'conflict' || e.state === 'missing'}
          <button type="button" class="danger" onclick={() => pushExternal(e, true)}>{t('editorOverwrite')}</button>
          <button type="button" onclick={() => stopExternal(e)} title={t('externalDiscardHint')}>{t('externalDiscard')}</button>
        {:else if e.state === 'error'}
          <button type="button" onclick={() => pushExternal(e, false)}>{t('retry')}</button>
          <button type="button" onclick={() => stopExternal(e)}>{t('externalClose')}</button>
        {:else if e.state !== 'uploading'}
          <button type="button" onclick={() => pushExternal(e, false)}>{t('externalUploadNow')}</button>
          <button type="button" onclick={() => stopExternal(e)} title={t('externalCloseHint')}>{t('externalClose')}</button>
        {/if}
      </div>
    {/each}
  </section>
{/if}

<style>
  .bar {
    background: var(--bg-elev);
    border-top: 1px solid var(--border);
    padding: 4px 8px 6px;
    font-size: 12px;
    flex: none;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 3px 0;
  }
  .item.changed,
  .item.conflict,
  .item.missing {
    background: color-mix(in srgb, #e5a000 12%, transparent);
  }
  .name {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 260px;
  }
  .status {
    color: var(--fg-dim);
    min-width: 0;
  }
  .item.error .status {
    color: var(--danger);
  }
  .spacer {
    flex: 1;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
  }
  button {
    padding: 1px 8px;
    white-space: nowrap;
  }
</style>
