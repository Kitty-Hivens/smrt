<script lang="ts">
  import { fly } from 'svelte/transition';
  import { toasts } from '../../lib/toasts.svelte';
  import { t } from '../../lib/i18n.svelte';
</script>

<!-- Fixed stack, bottom-right, above dialogs' scrim but below a modal: a notice
     must be readable while a dialog is open, and must never take the focus a
     dialog owns. -->
<div class="toaster" role="status" aria-live="polite">
  {#each toasts.list as n (n.id)}
    <div class="toast {n.kind}" transition:fly={{ y: 8, duration: 140 }}>
      <div class="body">
        <div class="text">{n.text}</div>
        {#if n.detail}<div class="detail mono">{n.detail}</div>{/if}
      </div>
      {#if n.action}
        <button class="act" onclick={n.action.run}>{n.action.label}</button>
      {/if}
      <button class="x" onclick={() => toasts.dismiss(n.id)} aria-label={t('common.close')}>×</button>
    </div>
  {/each}
</div>

<style>
  .toaster {
    position: fixed;
    right: var(--space-4);
    bottom: var(--space-4);
    z-index: 70;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--space-2);
    pointer-events: none;
    max-width: min(460px, calc(100vw - var(--space-5)));
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--seam);
    border-left-width: 3px;
    border-radius: var(--radius-sm);
    background: var(--panel-2);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-sm);
  }
  .toast.error {
    border-left-color: var(--danger);
  }
  .toast.ok {
    border-left-color: var(--ok);
  }
  .toast.info {
    border-left-color: var(--info);
  }
  .body {
    min-width: 0;
  }
  .text {
    line-height: 1.35;
  }
  .detail {
    margin-top: 3px;
    font-size: var(--fs-xs);
    color: var(--fg-dim);
    overflow-wrap: anywhere;
  }
  .act {
    flex-shrink: 0;
    padding: 4px 10px;
    font-size: var(--fs-sm);
  }
  .x {
    flex-shrink: 0;
    border: none;
    background: transparent;
    color: var(--fg-faint);
    font-size: var(--fs-lg);
    line-height: 1;
    padding: 2px 4px;
    cursor: pointer;
  }
  .x:hover {
    color: var(--fg);
  }
  @media (prefers-reduced-motion: reduce) {
    .toast {
      transition: none;
    }
  }
</style>
