<script lang="ts">
  import Modal from './Modal.svelte'
  import { t } from '../i18n'

  export interface HostKeyRequest {
    reqId: number
    host: string
    port: number
    keyType: string
    fingerprint: string
  }
  export interface HostKeyChange {
    host: string
    port: number
    expected: string
    got: string
  }

  /** Le finestre che una connessione può richiedere: host key nuova, host key cambiata, password. */
  let {
    hostKey,
    hostKeyChanged,
    askPassword,
    askKind,
    canSave,
    onreplyhostkey,
    ondismisshostkeychanged,
    onsubmitpassword,
    oncancelpassword,
    onmanagehostkeys,
  }: {
    hostKey: HostKeyRequest | null
    hostKeyChanged: HostKeyChange | null
    askPassword: boolean
    askKind: 'password' | 'passphrase'
    /** Si può salvare la password nella sessione (solo sessioni salvate). */
    canSave: boolean
    onreplyhostkey: (decision: 'once' | 'save' | 'reject') => void
    ondismisshostkeychanged: () => void
    onsubmitpassword: (password: string, save: boolean) => void
    oncancelpassword: () => void
    onmanagehostkeys?: () => void
  } = $props()

  let passwordInput = $state('')
  let saveChecked = $state(false)

  // Ogni richiesta riparte da un campo vuoto.
  $effect(() => {
    if (askPassword) {
      passwordInput = ''
      saveChecked = false
    }
  })

  function submit(e: Event) {
    e.preventDefault()
    const pw = passwordInput
    passwordInput = ''
    onsubmitpassword(pw, saveChecked)
  }
</script>

{#if askPassword}
  <Modal title={askKind === 'passphrase' ? t('passphrasePromptTitle') : t('passwordPromptTitle')} onclose={oncancelpassword}>
    <form id="pw-form" onsubmit={submit}>
      <input
        type="password"
        bind:value={passwordInput}
        autocomplete="off"
        aria-label={askKind === 'passphrase' ? t('passphrase') : t('password')}
      />
      {#if canSave}
        <label class="check"><input type="checkbox" bind:checked={saveChecked} /> {t('savePasswordToo')}</label>
      {/if}
    </form>
    {#snippet actions()}
      <button type="button" onclick={oncancelpassword}>{t('cancel')}</button>
      <button type="submit" form="pw-form" class="primary">{t('connect')}</button>
    {/snippet}
  </Modal>
{/if}

{#if hostKey}
  <Modal title={t('hostKeyTitle')} onclose={() => onreplyhostkey('reject')}>
    <p>{t('hostKeyBody')}</p>
    <dl>
      <dt>{t('host')}</dt>
      <dd>{hostKey.host}:{hostKey.port}</dd>
      <dt>{t('keyType')}</dt>
      <dd>{hostKey.keyType}</dd>
      <dt>{t('fingerprint')}</dt>
      <dd class="mono">{hostKey.fingerprint}</dd>
    </dl>
    {#snippet actions()}
      <button onclick={() => onreplyhostkey('reject')}>{t('cancel')}</button>
      <button onclick={() => onreplyhostkey('once')}>{t('acceptOnce')}</button>
      <button class="primary" onclick={() => onreplyhostkey('save')}>{t('accept')}</button>
    {/snippet}
  </Modal>
{/if}

{#if hostKeyChanged}
  <Modal title={t('hostKeyChangedTitle')} onclose={ondismisshostkeychanged}>
    <p class="alert">{t('hostKeyChangedBody')}</p>
    <dl>
      <dt>{t('host')}</dt>
      <dd>{hostKeyChanged.host}:{hostKeyChanged.port}</dd>
      <dt>{t('fingerprintSaved')}</dt>
      <dd class="mono">{hostKeyChanged.expected}</dd>
      <dt>{t('fingerprintReceived')}</dt>
      <dd class="mono">{hostKeyChanged.got}</dd>
    </dl>
    <p class="dim">{t('hostKeyChangedHelp')}</p>
    {#snippet actions()}
      {#if onmanagehostkeys}
        <button
          onclick={() => {
            ondismisshostkeychanged()
            onmanagehostkeys()
          }}>{t('manageHostKeys')}</button
        >
      {/if}
      <button class="primary" onclick={ondismisshostkeychanged}>{t('close')}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  dt {
    font-weight: 600;
    margin-top: 8px;
  }
  dd {
    margin: 0;
  }
  .mono {
    font-family: Consolas, monospace;
    word-break: break-all;
  }
  .alert {
    margin-top: 0;
    padding: 10px;
    border-radius: 4px;
    background: var(--danger);
    color: #fff;
    font-weight: 600;
  }
  .dim {
    color: var(--fg-dim);
  }
</style>
