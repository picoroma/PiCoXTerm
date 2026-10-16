<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { writeText } from '@tauri-apps/plugin-clipboard-manager'
  import { ABOUT, aboutImage } from '../about'
  import { t } from '../i18n'
  import Modal from './Modal.svelte'

  interface AboutInfo {
    name: string
    version: string
    commit: string
    buildDate: string
    tauriVersion: string
    webviewVersion: string | null
    os: string
    arch: string
    portable: boolean
    dataDir: string
    auditLog: string
  }

  let { onclose }: { onclose: () => void } = $props()

  let info = $state<AboutInfo | null>(null)
  let copied = $state(false)
  let closeBtn: HTMLButtonElement | undefined

  onMount(() => {
    closeBtn?.focus()
    invoke<AboutInfo>('about_info')
      .then((i) => (info = i))
      .catch(() => {})
  })

  const license = $derived(ABOUT.license || t('licenseTbd'))

  /** Testo per le segnalazioni: si copia con un clic. */
  function plainText(i: AboutInfo): string {
    return [
      `${i.name} ${i.version}`,
      `${t('build')}: ${i.commit} (${i.buildDate})`,
      `${t('author')}: ${ABOUT.author}`,
      `${t('license')}: ${license}`,
      `Tauri ${i.tauriVersion}, WebView2 ${i.webviewVersion ?? '?'}`,
      `${i.os} ${i.arch}`,
      `${t('mode')}: ${i.portable ? t('portableMode') : t('installedMode')}`,
      `${t('dataFolder')}: ${i.dataDir}`,
    ].join('\n')
  }

  async function copy() {
    if (!info) return
    await writeText(plainText(info))
    copied = true
    setTimeout(() => (copied = false), 2000)
  }
</script>

<Modal title={t('aboutTitle')} {onclose}>
  <div class="about">
    {#if aboutImage}
      <img class="pic" src={aboutImage} alt="" />
    {:else}
      <div class="pic placeholder" aria-hidden="true"><span>PXT</span></div>
    {/if}

    {#if ABOUT.incipit}
      <p class="incipit">{ABOUT.incipit}</p>
    {/if}

    <h3 class="title">PiCoXterm {info?.version ?? ''}</h3>
    <p class="desc">{t('aboutDescription')}</p>
    <p class="badges">{t('aboutBadges')}</p>

    <dl>
      <dt>{t('author')}</dt>
      <dd>{ABOUT.author}</dd>
      <dt>{t('version')}</dt>
      <dd>{info ? `${info.version}` : '…'}</dd>
      <dt>{t('build')}</dt>
      <dd>{info ? `${info.commit} · ${info.buildDate}` : '…'}</dd>
      <dt>{t('license')}</dt>
      <dd>{license}</dd>
      {#if info}
        <dt>{t('components')}</dt>
        <dd>Tauri {info.tauriVersion} · WebView2 {info.webviewVersion ?? '?'} · {info.os} {info.arch}</dd>
        <dt>{t('mode')}</dt>
        <dd>{info.portable ? t('portableMode') : t('installedMode')}</dd>
        <dt>{t('dataFolder')}</dt>
        <dd class="mono">{info.dataDir}</dd>
      {/if}
    </dl>
  </div>

  {#snippet actions()}
    <button type="button" disabled={!info} onclick={copy}>{copied ? t('copied') : t('copyInfo')}</button>
    <button type="button" class="primary" bind:this={closeBtn} onclick={onclose}>{t('close')}</button>
  {/snippet}
</Modal>

<style>
  .about {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    text-align: center;
  }
  .pic {
    width: 100%;
    max-height: 280px;
    object-fit: contain;
    border-radius: 6px;
  }
  .placeholder {
    height: 160px;
    display: grid;
    place-items: center;
    background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 55%, #000), var(--accent));
    color: #fff;
    font-size: 44px;
    font-weight: 700;
    letter-spacing: 4px;
  }
  .incipit {
    margin: 0;
    font-style: italic;
    color: var(--fg-dim);
    max-width: 440px;
  }
  .title {
    margin: 4px 0 0;
    font-size: 20px;
  }
  .desc {
    margin: 0;
    max-width: 440px;
    white-space: pre-line;
  }
  .badges {
    margin: 0;
    font-size: 12px;
    color: var(--fg-dim);
  }
  dl {
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: 4px 10px;
    margin: 8px 0 0;
    width: 100%;
    text-align: left;
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
    font-size: 12px;
  }
</style>
