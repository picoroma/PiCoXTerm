<script lang="ts">
  import { untrack } from 'svelte'
  import Modal from './Modal.svelte'
  import { t } from '../i18n'
  import type { Session } from './types'

  let {
    session,
    isNew,
    onsave,
    oncancel,
  }: {
    session: Session
    isNew: boolean
    onsave: (s: Session, password: string | null, clearPassword: boolean) => void
    oncancel: () => void
  } = $props()

  // Copia di lavoro: la sessione originale non viene toccata finché non si salva.
  const initial = untrack(() => $state.snapshot(session))
  let name = $state(initial.name)
  let host = $state(initial.host)
  let port = $state(initial.port)
  let username = $state(initial.username)
  let password = $state('')
  let clearPassword = $state(false)
  let notes = $state(initial.notes)
  let tags = $state(initial.ui.tags.join(', '))
  const hasStoredPassword = initial.credentialRef !== null

  const valid = $derived(
    host.trim() !== '' && username.trim() !== '' && Number.isInteger(port) && port >= 1 && port <= 65535,
  )

  function submit(e: Event) {
    e.preventDefault()
    if (!valid) return
    const updated: Session = {
      ...initial,
      name: name.trim() || host.trim(),
      host: host.trim(),
      port,
      username: username.trim(),
      notes,
      ui: {
        ...initial.ui,
        tags: tags
          .split(',')
          .map((x) => x.trim())
          .filter(Boolean),
      },
      updatedAt: new Date().toISOString(),
    }
    onsave(updated, password === '' ? null : password, clearPassword && password === '')
  }
</script>

<Modal title={isNew ? t('newSession') : t('editSession')} onclose={oncancel}>
  <form id="session-form" onsubmit={submit}>
    <label>{t('name')}<input bind:value={name} placeholder={host} /></label>
    <div class="row">
      <label class="grow">{t('host')}<input bind:value={host} required /></label>
      <label class="narrow">{t('port')}<input type="number" min="1" max="65535" bind:value={port} required /></label>
    </div>
    <label>{t('user')}<input bind:value={username} required autocomplete="off" /></label>
    <label>
      {t('password')}
      <input
        type="password"
        bind:value={password}
        autocomplete="new-password"
        placeholder={hasStoredPassword ? t('passwordSaved') : t('passwordNone')}
      />
    </label>
    {#if hasStoredPassword}
      <label class="check"><input type="checkbox" bind:checked={clearPassword} /> {t('clearPassword')}</label>
    {/if}
    <label>{t('tags')}<input bind:value={tags} placeholder="PRD, SAP" /></label>
    <label>{t('notes')}<textarea rows="3" bind:value={notes}></textarea></label>
  </form>
  {#snippet actions()}
    <button type="button" onclick={oncancel}>{t('cancel')}</button>
    <button type="submit" form="session-form" class="primary" disabled={!valid}>{t('save')}</button>
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
  }
  .row {
    display: flex;
    gap: 10px;
  }
  .grow {
    flex: 1;
  }
  .narrow {
    width: 90px;
  }
</style>
