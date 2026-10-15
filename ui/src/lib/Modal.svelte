<script lang="ts">
  import type { Snippet } from 'svelte'

  let {
    title,
    onclose,
    children,
    actions,
  }: {
    title: string
    onclose?: () => void
    children: Snippet
    actions?: Snippet
  } = $props()
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose?.()} />

<div class="backdrop">
  <div class="dialog" role="dialog" aria-modal="true" aria-label={title}>
    <h2>{title}</h2>
    {@render children()}
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: grid;
    place-items: center;
    z-index: 50;
  }
  .dialog {
    background: var(--bg-elev);
    padding: 20px;
    border-radius: 8px;
    width: min(520px, 92vw);
    max-height: 90vh;
    overflow: auto;
  }
  h2 {
    margin: 0 0 12px;
    font-size: 18px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
</style>
