<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { open, save } from '@tauri-apps/plugin-dialog'
  import ImportDialog from './ImportDialog.svelte'
  import Modal from './Modal.svelte'
  import { saveSettings, settings, type Settings } from './settings.svelte'
  import { THEME_NAMES, type ThemeName } from './themes'
  import { t } from '../i18n'

  let { onclose, onimported }: { onclose: () => void; onimported: () => void } = $props()

  // Copia di lavoro: si applica solo con "Salva".
  let draft = $state<Settings>(untrack(() => $state.snapshot(settings)))
  let error = $state('')
  let info = $state('')
  let importKind = $state<'putty' | 'mobaxterm' | null>(null)
  let paths = $state<{ dataDir: string; auditLog: string } | null>(null)
  interface KnownHost {
    host: string
    port: number
    keyType: string
    fingerprint: string
  }
  let knownHosts = $state<KnownHost[]>([])
  let confirmRemove = $state<string | null>(null)
  const khId = (k: KnownHost) => `${k.host}:${k.port}`

  async function loadKnownHosts() {
    try {
      knownHosts = await invoke<KnownHost[]>('known_hosts_list')
    } catch (e) {
      error = String(e)
    }
  }

  /// Due passaggi: il primo clic chiede conferma, il secondo rimuove.
  async function removeKnownHost(k: KnownHost) {
    if (confirmRemove !== khId(k)) {
      confirmRemove = khId(k)
      return
    }
    confirmRemove = null
    try {
      await invoke('known_hosts_remove', { host: k.host, port: k.port })
      await loadKnownHosts()
    } catch (e) {
      error = String(e)
    }
  }

  const jsonFilter = [{ name: 'JSON', extensions: ['json'] }]

  onMount(() => {
    void loadKnownHosts()
    invoke<{ dataDir: string; auditLog: string }>('app_paths')
      .then((p) => (paths = p))
      .catch(() => {})
  })

  async function exportSessions() {
    error = info = ''
    try {
      const path = await save({ defaultPath: 'picoxterm-sessions.json', filters: jsonFilter })
      if (!path) return
      const n = await invoke<number>('sessions_export', { path })
      info = t('exported', { n })
    } catch (e) {
      error = String(e)
    }
  }

  async function importSessions() {
    error = info = ''
    try {
      const path = await open({ multiple: false, directory: false, filters: jsonFilter })
      if (typeof path !== 'string') return
      const day = new Date().toISOString().slice(0, 10)
      const n = await invoke<number>('sessions_import', { path, folderName: `${t('importedFolder')} ${day}` })
      info = t('imported', { n })
      onimported()
    } catch (e) {
      error = String(e)
    }
  }

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
    <fieldset>
      <legend>{t('sessionsData')}</legend>
      <div class="row">
        <button type="button" onclick={exportSessions}>{t('exportSessions')}</button>
        <button type="button" onclick={importSessions}>{t('importSessions')}</button>
      </div>
      <div class="row">
        <button type="button" onclick={() => (importKind = 'putty')}>{t('importFromPutty')}</button>
        <button type="button" onclick={() => (importKind = 'mobaxterm')}>{t('importFromMoba')}</button>
      </div>
      <p class="hint">{t('exportHint')}</p>
      {#if paths}
        <p class="hint">{t('auditPath')}: <code>{paths.auditLog}</code></p>
      {/if}
    </fieldset>
    <fieldset>
      <legend>{t('knownHosts')}</legend>
      {#if knownHosts.length === 0}
        <p class="hint">{t('noKnownHosts')}</p>
      {/if}
      {#each knownHosts as k (k.host + k.port + k.keyType)}
        <div class="kh">
          <div class="grow">
            <strong>{k.host}:{k.port}</strong> <span class="hint">{k.keyType}</span>
            <div class="hint mono">{k.fingerprint}</div>
          </div>
          <button type="button" class:danger={confirmRemove === khId(k)} onclick={() => removeKnownHost(k)}>
            {confirmRemove === khId(k) ? t('confirmRemove') : t('removeHostKey')}
          </button>
        </div>
      {/each}
      <p class="hint">{t('knownHostsHint')}</p>
    </fieldset>
    {#if info}<p class="ok">{info}</p>{/if}
    {#if error}<p class="err">{error}</p>{/if}
  </form>
  {#snippet actions()}
    <button type="button" onclick={onclose}>{t('cancel')}</button>
    <button type="submit" form="settings-form" class="primary">{t('save')}</button>
  {/snippet}
</Modal>

{#if importKind}
  <ImportDialog
    kind={importKind}
    onclose={() => (importKind = null)}
    ondone={(n) => {
      importKind = null
      info = t('imported', { n })
      onimported()
    }}
  />
{/if}

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
  fieldset {
    border: 1px solid var(--border);
    border-radius: 4px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
    color: var(--fg-dim);
    font-size: 13px;
  }
  .hint {
    margin: 0;
    font-size: 12px;
    word-break: break-all;
  }
  .kh {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--fg);
  }
  .mono {
    font-family: Consolas, monospace;
  }
  .ok {
    color: #5cb85c;
    margin: 0;
  }
  .err {
    color: var(--danger);
    margin: 0;
  }
</style>
