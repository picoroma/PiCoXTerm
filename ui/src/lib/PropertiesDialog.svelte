<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { writeText } from '@tauri-apps/plugin-clipboard-manager'
  import Modal from './Modal.svelte'
  import { t } from '../i18n'
  import type { SftpEntry } from './types'

  interface TreeInfo {
    files: number
    dirs: number
    bytes: number
    partial: boolean
  }
  interface OpReport {
    done: number
    failed: { path: string; message: string }[]
  }

  let {
    entry,
    connId,
    onclose,
    onchanged,
  }: {
    entry: SftpEntry
    connId: number
    onclose: () => void
    /** Chiamata dopo una modifica dei permessi, per aggiornare l'elenco. */
    onchanged: () => void
  } = $props()

  const isDir = $derived(entry.kind === 'dir')
  const editable = $derived(entry.mode !== null && !entry.isSymlink)

  // Permessi: 9 bit rwx + 3 speciali (setuid, setgid, sticky).
  let mode = $state(0)
  let recursive = $state(false)
  let tree = $state<TreeInfo | null>(null)
  let treeError = $state('')
  let busy = $state(false)
  let message = $state('')
  let error = $state('')
  let octalText = $state('')
  /** Valore salvato sul server, per sapere se c'è qualcosa da applicare. */
  let applied = $state(0)

  onMount(() => {
    mode = (entry.mode ?? 0) & 0o7777
    applied = mode
    octalText = mode.toString(8).padStart(4, '0')
    if (isDir) {
      invoke<TreeInfo>('sftp_tree_info', { id: connId, path: entry.path })
        .then((r) => (tree = r))
        .catch((e) => (treeError = String(e)))
    }
  })

  const BITS = [
    { who: 'owner', shift: 6 },
    { who: 'group', shift: 3 },
    { who: 'other', shift: 0 },
  ] as const
  const PERMS = [
    { key: 'read', bit: 4 },
    { key: 'write', bit: 2 },
    { key: 'exec', bit: 1 },
  ] as const
  const SPECIAL = [
    { key: 'setuid', bit: 0o4000 },
    { key: 'setgid', bit: 0o2000 },
    { key: 'sticky', bit: 0o1000 },
  ] as const

  const has = (bit: number) => (mode & bit) !== 0
  function toggle(bit: number) {
    mode = has(bit) ? mode & ~bit : mode | bit
    octalText = mode.toString(8).padStart(4, '0')
  }
  function onOctal() {
    if (/^[0-7]{3,4}$/.test(octalText)) mode = parseInt(octalText, 8)
  }

  function symbolic(m: number): string {
    const kind = entry.kind === 'dir' ? 'd' : entry.isSymlink ? 'l' : '-'
    const rwx = (shift: number, special: number, ch: string) => {
      const v = (m >> shift) & 7
      let x = v & 1 ? 'x' : '-'
      if (special) x = v & 1 ? ch : ch.toUpperCase()
      return `${v & 4 ? 'r' : '-'}${v & 2 ? 'w' : '-'}${x}`
    }
    return kind + rwx(6, m & 0o4000, 's') + rwx(3, m & 0o2000, 's') + rwx(0, m & 0o1000, 't')
  }

  function size(n: number): string {
    if (n < 1024) return `${n} B`
    const units = ['KiB', 'MiB', 'GiB', 'TiB']
    let v = n
    let u = -1
    do {
      v /= 1024
      u++
    } while (v >= 1024 && u < units.length - 1)
    return `${v.toFixed(v < 10 ? 1 : 0)} ${units[u]} (${n.toLocaleString()} B)`
  }

  const date = (secs: number | null) => (secs === null ? '—' : new Date(secs * 1000).toLocaleString())
  const owner = $derived(
    [entry.owner ?? (entry.uid !== null ? String(entry.uid) : null), entry.group ?? (entry.gid !== null ? String(entry.gid) : null)]
      .filter((x) => x !== null)
      .join(' : ') || '—',
  )
  const typeLabel = $derived(
    entry.isSymlink ? t('typeLink') : entry.kind === 'dir' ? t('typeFolder') : entry.kind === 'file' ? t('typeFile') : t('typeOther'),
  )

  async function apply() {
    busy = true
    message = error = ''
    try {
      const deep = isDir && recursive
      // I bit speciali (setuid, setgid, sticky) valgono solo per l'elemento scelto: darli a tutto il
      // contenuto, come farebbe `chmod -R 4755`, sarebbe pericoloso.
      const special = mode & 0o7000
      const r = await invoke<OpReport>('sftp_chmod', { id: connId, path: entry.path, mode: deep ? mode & 0o777 : mode, recursive: deep })
      if (deep && special !== 0) {
        const top = await invoke<OpReport>('sftp_chmod', { id: connId, path: entry.path, mode, recursive: false })
        r.failed.push(...top.failed)
      }
      if (r.failed.length > 0) {
        error = t('chmodPartial', { n: r.failed.length, first: r.failed[0].message })
      } else {
        message = t('chmodDone', { n: r.done })
        applied = mode
      }
      onchanged()
    } catch (e) {
      error = String(e)
    } finally {
      busy = false
    }
  }
</script>

<Modal title={t('properties')} {onclose}>
  <dl>
    <dt>{t('name')}</dt>
    <dd><strong>{entry.name}</strong></dd>
    <dt>{t('path')}</dt>
    <dd class="mono path">
      {entry.path}
      <button type="button" class="mini" title={t('copyPath')} aria-label={t('copyPath')} onclick={() => writeText(entry.path)}>⧉</button>
    </dd>
    <dt>{t('type')}</dt>
    <dd>{typeLabel}</dd>
    <dt>{t('size')}</dt>
    <dd>
      {#if isDir}
        {#if tree}
          {t('treeSummary', { files: tree.files, dirs: Math.max(0, tree.dirs - 1), size: size(tree.bytes) })}{tree.partial ? ` (${t('partial')})` : ''}
        {:else if treeError}
          <span class="err">{treeError}</span>
        {:else}
          {t('calculating')}
        {/if}
      {:else}
        {size(entry.size)}
      {/if}
    </dd>
    <dt>{t('modified')}</dt>
    <dd>{date(entry.modified)}</dd>
    <dt>{t('ownerGroup')}</dt>
    <dd>{owner}</dd>
  </dl>

  <fieldset disabled={!editable || busy}>
    <legend>{t('permissions')} <span class="mono">{symbolic(mode)}</span></legend>
    {#if !editable}
      <p class="dim">{entry.isSymlink ? t('permsLink') : t('permsUnknown')}</p>
    {/if}
    <table>
      <thead>
        <tr><th></th><th>{t('permRead')}</th><th>{t('permWrite')}</th><th>{t('permExec')}</th></tr>
      </thead>
      <tbody>
        {#each BITS as row (row.who)}
          <tr>
            <th scope="row">{t(`perm_${row.who}`)}</th>
            {#each PERMS as p (p.key)}
              <td><input type="checkbox" aria-label={`${t(`perm_${row.who}`)} ${t(`perm${p.key[0].toUpperCase()}${p.key.slice(1)}`)}`} checked={has(p.bit << row.shift)} onchange={() => toggle(p.bit << row.shift)} /></td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>
    <div class="special">
      {#each SPECIAL as s (s.key)}
        <label class="check"><input type="checkbox" checked={has(s.bit)} onchange={() => toggle(s.bit)} /> {t(`perm_${s.key}`)}</label>
      {/each}
    </div>
    <label class="octal">
      {t('octal')}
      <input bind:value={octalText} oninput={onOctal} maxlength="4" size="5" inputmode="numeric" aria-label={t('octal')} />
    </label>
    {#if isDir}
      <label class="check">
        <input type="checkbox" bind:checked={recursive} /> {t('applyToContents')}
      </label>
      {#if recursive}
        <p class="warn">{t('applyToContentsWarn')}{(mode & 0o7000) !== 0 ? ` ${t('applyToContentsSpecial')}` : ''}</p>
      {/if}
    {/if}
  </fieldset>
  {#if message}<p class="ok" role="status">{message}</p>{/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}

  {#snippet actions()}
    <button type="button" onclick={onclose}>{t('close')}</button>
    <button type="button" class="primary" disabled={!editable || busy || (mode === applied && !recursive)} onclick={apply}>
      {busy ? t('applying') : t('applyPermissions')}
    </button>
  {/snippet}
</Modal>

<style>
  dl {
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: 4px 10px;
    margin: 0 0 12px;
    font-size: 13px;
  }
  dt {
    color: var(--fg-dim);
  }
  dd {
    margin: 0;
    word-break: break-all;
  }
  .mono {
    font-family: Consolas, monospace;
  }
  .path {
    display: flex;
    gap: 6px;
    align-items: flex-start;
  }
  .mini {
    padding: 0 6px;
    flex: none;
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: 4px;
    margin: 0;
    padding: 8px 10px;
  }
  table {
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 2px 12px 2px 0;
    text-align: left;
    font-weight: normal;
  }
  thead th {
    color: var(--fg-dim);
  }
  .special,
  .octal {
    display: flex;
    gap: 12px;
    align-items: center;
    margin-top: 8px;
    font-size: 13px;
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .octal input {
    font-family: Consolas, monospace;
    width: 64px;
  }
  .dim {
    color: var(--fg-dim);
    font-size: 12px;
    margin: 0 0 6px;
  }
  .warn {
    margin: 6px 0 0;
    padding: 6px 8px;
    border: 1px solid #f0ad4e;
    border-radius: 4px;
    font-size: 12px;
  }
  .ok {
    color: #5cb85c;
    margin: 8px 0 0;
  }
  .err {
    color: var(--danger);
    margin: 8px 0 0;
  }
</style>
