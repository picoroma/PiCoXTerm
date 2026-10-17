<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { t } from '../i18n'
  import type { SftpEntry, TextFile } from './types'

  interface ObjectInfo {
    size: number
    etag: string | null
    modified: number | null
    storageClass: string | null
    contentType: string | null
    cacheControl: string | null
    contentDisposition: string | null
    contentEncoding: string | null
    metadata: Record<string, string>
    versionId: string | null
    encryption: string | null
    tags: [string, string][]
  }

  let {
    connId,
    selection,
    onchanged,
  }: {
    connId: number
    /** Voci selezionate nel pannello dei file. */
    selection: SftpEntry[]
    /** Un'operazione ha cambiato l'oggetto (data, dimensione…): il pannello può aggiornarsi. */
    onchanged?: () => void
  } = $props()

  type Tab = 'props' | 'headers' | 'tags' | 'preview'
  let tab = $state<Tab>('props')
  let info = $state<ObjectInfo | null>(null)
  let loading = $state(false)
  let error = $state('')
  let notice = $state('')

  // Copie di lavoro dei moduli.
  let contentType = $state('')
  let cacheControl = $state('')
  let disposition = $state('')
  let encoding = $state('')
  let meta = $state<{ key: string; value: string }[]>([])
  let tags = $state<{ key: string; value: string }[]>([])
  let storageClass = $state('STANDARD')
  let busy = $state(false)

  const CLASSES = ['STANDARD', 'STANDARD_IA', 'ONEZONE_IA', 'INTELLIGENT_TIERING', 'GLACIER_IR', 'GLACIER', 'DEEP_ARCHIVE']

  const target = $derived(selection.length === 1 && selection[0].kind === 'file' ? selection[0] : null)

  let seq = 0
  $effect(() => {
    const path = target?.path
    info = null
    error = ''
    notice = ''
    preview = null
    previewError = ''
    if (!path) return
    const mine = ++seq
    loading = true
    invoke<ObjectInfo>('s3_object_info', { id: connId, path })
      .then((r) => {
        if (mine !== seq) return
        info = r
        contentType = r.contentType ?? ''
        cacheControl = r.cacheControl ?? ''
        disposition = r.contentDisposition ?? ''
        encoding = r.contentEncoding ?? ''
        meta = Object.entries(r.metadata).map(([key, value]) => ({ key, value }))
        tags = r.tags.map(([key, value]) => ({ key, value }))
        storageClass = r.storageClass ?? 'STANDARD'
      })
      .catch((e) => {
        if (mine === seq) error = String(e)
      })
      .finally(() => {
        if (mine === seq) loading = false
      })
  })

  const MAX_PREVIEW = 5 * 1024 * 1024
  let preview = $state<TextFile | null>(null)
  let previewError = $state('')
  let previewLoading = $state(false)

  $effect(() => {
    if (tab !== 'preview' || !target || preview || previewError) return
    if (target.size > MAX_PREVIEW) {
      previewError = t('s3PreviewTooBig')
      return
    }
    const mine = seq
    previewLoading = true
    invoke<TextFile>('sftp_read_text', { id: connId, path: target.path })
      .then((r) => {
        if (mine === seq) preview = r
      })
      .catch(() => {
        if (mine === seq) previewError = t('s3PreviewNone')
      })
      .finally(() => (previewLoading = false))
  })

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`
    const u = ['KiB', 'MiB', 'GiB', 'TiB']
    let v = n / 1024
    let i = 0
    while (v >= 1024 && i < u.length - 1) {
      v /= 1024
      i++
    }
    return `${v.toFixed(1)} ${u[i]}`
  }

  async function apply(action: () => Promise<unknown>) {
    busy = true
    error = ''
    notice = ''
    try {
      await action()
      notice = t('s3Applied')
      onchanged?.()
    } catch (e) {
      error = String(e)
    } finally {
      busy = false
    }
  }

  const saveHeaders = () =>
    apply(() =>
      invoke('s3_set_headers', {
        id: connId,
        path: target!.path,
        headers: {
          contentType,
          cacheControl,
          contentDisposition: disposition,
          contentEncoding: encoding,
          metadata: Object.fromEntries(meta.filter((m) => m.key.trim()).map((m) => [m.key.trim(), m.value])),
        },
      }),
    )

  const saveTags = () =>
    apply(() =>
      invoke('s3_set_tags', {
        id: connId,
        path: target!.path,
        tags: tags.filter((x) => x.key.trim()).map((x) => [x.key.trim(), x.value]),
      }),
    )

  const saveClass = () =>
    apply(() => invoke('s3_set_storage_class', { id: connId, path: target!.path, class: storageClass }))
</script>

<div class="detail">
  <div class="tabs" role="tablist">
    {#each [['props', 's3TabProps'], ['headers', 's3TabHeaders'], ['tags', 's3TabTags'], ['preview', 's3TabPreview']] as [id, label] (id)}
      <button role="tab" aria-selected={tab === id} class:on={tab === id} onclick={() => (tab = id as Tab)}>{t(label)}</button>
    {/each}
  </div>
  <div class="body">
    {#if selection.length === 0}
      <p class="dim">{t('s3NoSelection')}</p>
    {:else if selection.length > 1}
      <p class="dim">{t('s3ManySelected', { n: selection.length })}</p>
    {:else if !target}
      <p class="dim">{t('s3FolderSelected')}</p>
    {:else}
      <h3 title={target.path}>{target.name}</h3>
      {#if loading}<p class="dim">{t('s3Loading')}</p>{/if}
      {#if error}<p class="error">{error}</p>{/if}
      {#if info}
        {#if tab === 'props'}
          <dl>
            <dt>{t('s3Size')}</dt><dd>{fmtBytes(info.size)} ({info.size} B)</dd>
            <dt>{t('s3Modified')}</dt><dd>{info.modified ? new Date(info.modified * 1000).toLocaleString() : '—'}</dd>
            <dt>{t('s3Etag')}</dt><dd class="mono">{info.etag ?? '—'}</dd>
            <dt>{t('s3Version')}</dt><dd class="mono">{info.versionId ?? '—'}</dd>
            <dt>{t('s3Encryption')}</dt><dd>{info.encryption ?? '—'}</dd>
            <dt>{t('s3Class')}</dt>
            <dd>
              <select bind:value={storageClass} disabled={busy}>
                {#each CLASSES as c (c)}<option value={c}>{c}</option>{/each}
                {#if !CLASSES.includes(storageClass)}<option value={storageClass}>{storageClass}</option>{/if}
              </select>
              <button type="button" disabled={busy || storageClass === (info.storageClass ?? 'STANDARD')} onclick={saveClass}>{t('s3ChangeClass')}</button>
            </dd>
          </dl>
        {:else if tab === 'headers'}
          <label>{t('s3ContentType')}<input bind:value={contentType} spellcheck="false" /></label>
          <label>{t('s3CacheControl')}<input bind:value={cacheControl} spellcheck="false" /></label>
          <label>{t('s3Disposition')}<input bind:value={disposition} spellcheck="false" /></label>
          <label>{t('s3Encoding')}<input bind:value={encoding} spellcheck="false" /></label>
          <fieldset>
            <legend>{t('s3Metadata')}</legend>
            {#each meta as row, i (i)}
              <div class="kv">
                <input bind:value={row.key} placeholder={t('s3TagKey')} spellcheck="false" />
                <input bind:value={row.value} placeholder={t('s3TagValue')} spellcheck="false" />
                <button type="button" aria-label={t('remove')} onclick={() => (meta = meta.filter((_, j) => j !== i))}>✕</button>
              </div>
            {/each}
            <button type="button" onclick={() => (meta = [...meta, { key: '', value: '' }])}>{t('s3AddRow')}</button>
          </fieldset>
          <p class="dim small">{t('s3HeadersNote')}</p>
          <button type="button" class="primary" disabled={busy} onclick={saveHeaders}>{t('s3Apply')}</button>
        {:else if tab === 'tags'}
          {#each tags as row, i (i)}
            <div class="kv">
              <input bind:value={row.key} placeholder={t('s3TagKey')} spellcheck="false" />
              <input bind:value={row.value} placeholder={t('s3TagValue')} spellcheck="false" />
              <button type="button" aria-label={t('remove')} onclick={() => (tags = tags.filter((_, j) => j !== i))}>✕</button>
            </div>
          {/each}
          <div class="row">
            <button type="button" onclick={() => (tags = [...tags, { key: '', value: '' }])}>{t('s3AddRow')}</button>
            <button type="button" class="primary" disabled={busy} onclick={saveTags}>{t('s3Apply')}</button>
          </div>
        {:else if previewLoading}
          <p class="dim">{t('s3Loading')}</p>
        {:else if previewError}
          <p class="dim">{previewError}</p>
        {:else if preview}
          <pre class="preview">{preview.content}</pre>
        {/if}
        {#if notice}<p class="ok">{notice}</p>{/if}
      {/if}
    {/if}
  </div>
</div>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    border-left: 1px solid var(--border);
    background: var(--bg-elev);
  }
  .tabs {
    display: flex;
    border-bottom: 1px solid var(--border);
  }
  .tabs button {
    flex: 1;
    padding: 6px 4px;
    border: 0;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
    font-size: 12px;
  }
  .tabs button.on {
    border-bottom-color: var(--accent);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 13px;
  }
  h3 {
    margin: 0;
    font-size: 14px;
    word-break: break-all;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 10px;
    margin: 0;
  }
  dt {
    color: var(--fg-dim);
  }
  dd {
    margin: 0;
    word-break: break-all;
  }
  .mono,
  pre {
    font-family: Consolas, monospace;
    font-size: 12px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    color: var(--fg-dim);
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: 4px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    color: var(--fg-dim);
  }
  .kv {
    display: grid;
    grid-template-columns: 1fr 1fr auto;
    gap: 4px;
  }
  .kv input {
    min-width: 0;
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .dim {
    color: var(--fg-dim);
    margin: 0;
  }
  .small {
    font-size: 12px;
  }
  .error {
    color: #ffb3b3;
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .ok {
    color: #7fd17f;
    margin: 0;
  }
  .preview {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
