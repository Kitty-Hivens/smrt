<script lang="ts">
  import { dialogs } from '../lib/dialogs.svelte';
  import { t } from '../lib/i18n.svelte';

  const a = $derived(dialogs.active);
  let value = $state('');
  let input = $state<HTMLInputElement | null>(null);
  let okBtn = $state<HTMLButtonElement | null>(null);
  let dialogEl = $state<HTMLElement | null>(null);

  // Seed + focus when a dialog opens: the field for a prompt, else the primary
  // action for a confirm.
  $effect(() => {
    if (!a) return;
    if (a.kind === 'prompt') {
      value = a.initial;
      input?.focus();
    } else {
      okBtn?.focus();
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
    if (e.key === 'Escape') {
      cancel();
      return;
    }
    if (e.key === 'Enter' && a.kind === 'prompt') {
      accept();
      return;
    }
    // Trap Tab inside the dialog so focus can't reach the obscured page behind it.
    if (e.key === 'Tab' && dialogEl) {
      const items = [...dialogEl.querySelectorAll<HTMLElement>('button, input')];
      if (items.length === 0) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }
</script>

<svelte:window onkeydown={onKey} />

{#if a}
  <div
    class="overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) cancel();
    }}
  >
    <div
      class="dlg panel"
      bind:this={dialogEl}
      role="dialog"
      aria-modal="true"
      aria-labelledby="dialog-title"
    >
      <h3 id="dialog-title">{a.title}</h3>
      {#if a.kind === 'confirm'}
        <p class="msg">{a.message}</p>
      {:else}
        <label class="fld">
          {a.label}
          <input bind:this={input} bind:value placeholder={a.placeholder} />
        </label>
      {/if}
      <div class="actions">
        <button onclick={cancel}>{t('dialog.cancel')}</button>
        <button
          bind:this={okBtn}
          class="primary"
          class:danger={a.kind === 'confirm' && a.danger}
          onclick={accept}
        >
          {a.kind === 'confirm' && a.danger ? t('dialog.delete') : t('dialog.ok')}
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
