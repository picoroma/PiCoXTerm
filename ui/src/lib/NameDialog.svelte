<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import Modal from './Modal.svelte'
  import { t } from '../i18n'

  let {
    title,
    label,
    initial = '',
    confirmLabel,
    error = '',
    onconfirm,
    oncancel,
  }: {
    title: string
    label: string
    initial?: string
    confirmLabel: string
    /** Errore del tentativo precedente (per esempio "esiste già"). */
    error?: string
    onconfirm: (name: string) => void
    oncancel: () => void
  } = $props()

  let name = $state(untrack(() => initial))
  let input: HTMLInputElement | undefined
  const trimmed = $derived(name.trim())
  const invalid = $derived(trimmed === '' || trimmed === '.' || trimmed === '..' || trimmed.includes('/'))

  onMount(() => {
    input?.focus()
    // Per la rinomina si seleziona il nome senza estensione, come negli esploratori di file.
    const dot = initial.lastIndexOf('.')
    input?.setSelectionRange(0, dot > 0 ? dot : initial.length)
  })

  function submit(e: Event) {
    e.preventDefault()
    if (!invalid) onconfirm(trimmed)
  }
</script>

<Modal {title} onclose={oncancel}>
  <form id="name-form" onsubmit={submit}>
    <label>
      {label}
      <input bind:this={input} bind:value={name} spellcheck="false" autocomplete="off" />
    </label>
    {#if trimmed.includes('/')}<p class="err">{t('nameNoSlash')}</p>{/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </form>
  {#snippet actions()}
    <button type="button" onclick={oncancel}>{t('cancel')}</button>
    <button type="submit" form="name-form" class="primary" disabled={invalid}>{confirmLabel}</button>
  {/snippet}
</Modal>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
    color: var(--fg-dim);
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: 13px;
  }
</style>
