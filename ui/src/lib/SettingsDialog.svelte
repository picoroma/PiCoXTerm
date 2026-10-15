<script lang="ts">
  import { untrack } from 'svelte'
  import Modal from './Modal.svelte'
  import { saveSettings, settings, type Settings } from './settings.svelte'
  import { THEME_NAMES, type ThemeName } from './themes'
  import { t } from '../i18n'

  let { onclose }: { onclose: () => void } = $props()

  // Copia di lavoro: si applica solo con "Salva".
  let draft = $state<Settings>(untrack(() => $state.snapshot(settings)))
  let error = $state('')

  async function submit(e: Event) {
    e.preventDefault()
    try {
      await saveSettings($state.snapshot(draft))
      onclose()
    } catch (err) {
      error = String(err)
    }
  }
</script>

<Modal title={t('settings')} {onclose}>
  <form id="settings-form" onsubmit={submit}>
    <label>
      {t('theme')}
      <select bind:value={draft.theme}>
        {#each THEME_NAMES as name (name)}
          <option value={name}>{t(`theme_${name}`)}</option>
        {/each}
      </select>
    </label>
    <label>{t('fontFamily')}<input bind:value={draft.fontFamily} /></label>
    <div class="row">
      <label class="grow">{t('fontSize')}<input type="number" min="8" max="32" bind:value={draft.fontSize} /></label>
      <label class="grow">{t('scrollback')}<input type="number" min="1000" max="200000" step="1000" bind:value={draft.scrollback} /></label>
    </div>
    <label class="check"><input type="checkbox" bind:checked={draft.copyOnSelect} /> {t('copyOnSelect')}</label>
    <label class="check"><input type="checkbox" bind:checked={draft.rightClickPaste} /> {t('rightClickPaste')}</label>
    <label class="check"><input type="checkbox" bind:checked={draft.pasteMultilineWarning} /> {t('pasteWarning')}</label>
    {#if error}<p class="err">{error}</p>{/if}
  </form>
  {#snippet actions()}
    <button type="button" onclick={onclose}>{t('cancel')}</button>
    <button type="submit" form="settings-form" class="primary">{t('save')}</button>
  {/snippet}
</Modal>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
    color: var(--fg-dim);
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    color: var(--fg);
  }
  .row {
    display: flex;
    gap: 10px;
  }
  .grow {
    flex: 1;
  }
  .err {
    color: var(--danger);
    margin: 0;
  }
</style>
