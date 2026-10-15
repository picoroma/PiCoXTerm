<script lang="ts">
  import { untrack } from 'svelte'
  import { homeDir, join } from '@tauri-apps/api/path'
  import { open } from '@tauri-apps/plugin-dialog'
  import Modal from './Modal.svelte'
  import { t } from '../i18n'
  import type { AuthMethod, Session } from './types'

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
  let method = $state<AuthMethod>(initial.auth.method)
  let keyPath = $state(initial.auth.keyPath ?? '')
  let secret = $state('')
  let clearSecret = $state(false)
  let notes = $state(initial.notes)
  let tags = $state(initial.ui.tags.join(', '))
  const hasStoredSecret = initial.credentialRef !== null

  const usesSecret = $derived(method === 'password' || method === 'key')
  const valid = $derived(
    host.trim() !== '' &&
      username.trim() !== '' &&
      Number.isInteger(port) &&
      port >= 1 &&
      port <= 65535 &&
      (method !== 'key' || keyPath.trim() !== ''),
  )

  async function browse() {
    let defaultPath: string | undefined
    try {
      defaultPath = await join(await homeDir(), '.ssh')
    } catch {
      defaultPath = undefined
    }
    const picked = await open({ multiple: false, directory: false, defaultPath, title: t('keyFile') })
    if (typeof picked === 'string') keyPath = picked
  }

  function submit(e: Event) {
    e.preventDefault()
    if (!valid) return
    const updated: Session = {
      ...initial,
      name: name.trim() || host.trim(),
      host: host.trim(),
      port,
      username: username.trim(),
      auth: { method, keyPath: method === 'key' ? keyPath.trim() : null },
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
    onsave(updated, secret === '' ? null : secret, clearSecret && secret === '')
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
      {t('authMethod')}
      <select bind:value={method}>
        <option value="password">{t('methodPassword')}</option>
        <option value="key">{t('methodKey')}</option>
        <option value="agent">{t('methodAgent')}</option>
      </select>
    </label>
    {#if method === 'key'}
      <label>
        {t('keyFile')}
        <span class="row">
          <input class="grow" bind:value={keyPath} placeholder="~/.ssh/id_ed25519" required />
          <button type="button" onclick={browse}>{t('browse')}</button>
        </span>
      </label>
    {/if}
    {#if method === 'agent'}
      <p class="hint">{t('agentHint')}</p>
    {/if}
    {#if usesSecret}
      <label>
        {method === 'key' ? t('passphrase') : t('password')}
        <input
          type="password"
          bind:value={secret}
          autocomplete="new-password"
          placeholder={hasStoredSecret
            ? t('secretSaved')
            : method === 'key'
              ? t('passphraseNone')
              : t('passwordNone')}
        />
      </label>
    {/if}
    {#if hasStoredSecret}
      <label class="check"><input type="checkbox" bind:checked={clearSecret} /> {t('clearPassword')}</label>
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
  .hint {
    margin: 0;
    font-size: 13px;
    color: var(--fg-dim);
  }
</style>
