<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import Icon from './Icon.svelte'
  import Modal from './Modal.svelte'
  import { PROTOCOLS, type Protocol } from './protocols'
  import { t } from '../i18n'

  let { onpick, onclose }: { onpick: (protocol: Protocol) => void; onclose: () => void } = $props()

  let grid = $state<HTMLDivElement>()
  /** Protocolli vietati dalla policy dell'organizzazione (se c'è un elenco, solo quelli sono consentiti). */
  let allowed = $state<string[] | null>(null)

  onMount(() => {
    invoke<{ policy: { allowedProtocols: string[] | null } }>('policy_get')
      .then((r) => (allowed = r.policy.allowedProtocols))
      .catch(() => {})
    grid?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
  })
  const forbidden = (id: string) => allowed !== null && !allowed.includes(id)
</script>

<Modal title={t('newSessionTitle')} {onclose} width="min(640px, 94vw)">
  <p class="hint">{t('newSessionHint')}</p>
  <div class="grid" bind:this={grid} role="group" aria-label={t('newSessionTitle')}>
    {#each PROTOCOLS as p (p.id)}
      <button
        type="button"
        class="tile"
        disabled={!p.available || forbidden(p.id)}
        onclick={() => onpick(p.id)}
        aria-label={`${t(`proto_${p.id}`)}${p.available ? '' : ` (${t('soon')})`}`}
      >
        <span class="ico"><Icon name={p.icon} size={30} /></span>
        <strong>{t(`proto_${p.id}`)}</strong>
        <span class="desc">{t(`proto_${p.id}_desc`)}</span>
        {#if !p.available}<span class="soon">{t('soon')}</span>{:else if forbidden(p.id)}<span class="soon">{t('byPolicy')}</span>{/if}
      </button>
    {/each}
  </div>

  {#snippet actions()}
    <button type="button" onclick={onclose}>{t('cancel')}</button>
  {/snippet}
</Modal>

<style>
  .hint {
    margin: 0 0 12px;
    color: var(--fg-dim);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
  }
  .tile {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 16px 10px 14px;
    text-align: center;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    color: var(--fg);
    cursor: pointer;
  }
  .tile:hover:not(:disabled),
  .tile:focus-visible {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, var(--bg));
    outline: none;
  }
  .tile:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .ico {
    color: var(--accent);
    display: grid;
    place-items: center;
    height: 36px;
  }
  .tile:disabled .ico {
    color: var(--fg-dim);
  }
  strong {
    font-size: 15px;
  }
  .desc {
    font-size: 12px;
    color: var(--fg-dim);
    line-height: 1.3;
  }
  .soon {
    position: absolute;
    top: 6px;
    right: 6px;
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 8px;
    background: var(--bg-hover);
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
</style>
