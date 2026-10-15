<script lang="ts">
  import Modal from './Modal.svelte'
  import { t } from '../i18n'
  import type { ConflictPolicy, PlanSummary } from './transfers.svelte'

  let {
    plan,
    onstart,
    oncancel,
  }: {
    plan: PlanSummary
    onstart: (policy: ConflictPolicy) => void
    oncancel: () => void
  } = $props()

  // Scelta di partenza non distruttiva: i file esistenti restano com'erano.
  let policy = $state<ConflictPolicy>('skip')

  const options: { value: ConflictPolicy; label: string; hint: string }[] = [
    { value: 'skip', label: 'policySkip', hint: 'policySkipHint' },
    { value: 'overwrite', label: 'policyOverwrite', hint: 'policyOverwriteHint' },
    { value: 'overwriteIfNewer', label: 'policyNewer', hint: 'policyNewerHint' },
    { value: 'rename', label: 'policyRename', hint: 'policyRenameHint' },
  ]
</script>

<Modal title={t('conflictsTitle')} onclose={oncancel}>
  <p>{t('conflictsBody', { n: plan.conflicts, total: plan.fileCount })}</p>
  {#if plan.conflictSamples.length > 0}
    <p class="samples">
      {plan.conflictSamples.join(', ')}{plan.conflicts > plan.conflictSamples.length ? '…' : ''}
    </p>
  {/if}
  <fieldset>
    <legend>{t('conflictsApplyAll')}</legend>
    {#each options as o (o.value)}
      <label class="opt">
        <input type="radio" name="policy" value={o.value} bind:group={policy} />
        <span><strong>{t(o.label)}</strong><br /><small>{t(o.hint)}</small></span>
      </label>
    {/each}
  </fieldset>
  {#snippet actions()}
    <button type="button" onclick={oncancel}>{t('cancel')}</button>
    <button type="button" class="primary" onclick={() => onstart(policy)}>{t('startTransfer')}</button>
  {/snippet}
</Modal>

<style>
  .samples {
    font-family: Consolas, monospace;
    font-size: 12px;
    color: var(--fg-dim);
    word-break: break-all;
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: 4px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
  }
  .opt {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    cursor: pointer;
  }
  small {
    color: var(--fg-dim);
  }
</style>
