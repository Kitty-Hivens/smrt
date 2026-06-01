<script lang="ts">
  import { dialogs } from '../lib/dialogs.svelte';

  const a = $derived(dialogs.active);
  let value = $state('');
  let input = $state<HTMLInputElement | null>(null);

  // Seed the field + focus when a prompt opens.
  $effect(() => {
    if (a?.kind === 'prompt') {
      value = a.initial;
      input?.focus();
    }
  });

  function cancel() {
    if (!a) return;
    if (a.kind === 'confirm') dialogs.resolveConfirm(false);
    else dialogs.resolvePrompt(null);
  }
  function accept() {
    if (!a) return;
    if (a.kind === 'confirm') dialogs.resolveConfirm(true);
    else dialogs.resolvePrompt(value.trim() || null);
  }
  function onKey(e: KeyboardEvent) {
    if (!a) return;
    if (e.key === 'Escape') cancel();
    else if (e.key === 'Enter' && a.kind === 'prompt') accept();
  }
</script>

<svelte:window onkeydown={onKey} />

{#if a}
  <div class="overlay" onclick={cancel} role="presentation">
    <div class="dlg panel" onclick={(e) => e.stopPropagation()} role="presentation">
      <h3>{a.title}</h3>
      {#if a.kind === 'confirm'}
        <p class="msg">{a.message}</p>
      {:else}
        <label class="fld">
          {a.label}
          <input bind:this={input} bind:value placeholder={a.placeholder} />
        </label>
      {/if}
      <div class="actions">
        <button onclick={cancel}>Cancel</button>
        <button
          class="primary"
          class:danger={a.kind === 'confirm' && a.danger}
          onclick={accept}
        >
          {a.kind === 'confirm' && a.danger ? 'Delete' : 'OK'}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: grid;
    place-items: center;
    z-index: 80;
  }
  .dlg {
    width: 440px;
    max-width: 92vw;
    padding: 20px;
  }
  h3 {
    margin: 0 0 12px;
    font-size: 14px;
    color: var(--fg);
  }
  .msg {
    margin: 0 0 18px;
    color: var(--fg-dim);
    font-size: 13px;
    line-height: 1.5;
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
    color: var(--fg-dim);
    margin-bottom: 18px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
  }
  .danger {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
