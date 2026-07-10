<script lang="ts">
  import { Dialog } from 'bits-ui';
  import { dialogs } from '../lib/dialogs.svelte';
  import { t } from '../lib/i18n.svelte';

  // Bits UI Dialog gives focus-trap, escape-to-close, dismiss-on-outside-click,
  // scroll-lock, portal and aria wiring for free -- replacing the hand-rolled
  // overlay + manual Tab/Escape trapping this file used to carry.
  const a = $derived(dialogs.active);
  let value = $state('');

  // seed the prompt field when a prompt opens
  $effect(() => {
    if (a?.kind === 'prompt') value = a.initial;
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
  // escape / outside-click / close all flip Bits' open to false; settle the
  // pending dialog as a cancel so its awaiting caller never hangs.
  function onOpenChange(open: boolean) {
    if (!open && a) cancel();
  }
</script>

<Dialog.Root open={!!a} {onOpenChange}>
  <Dialog.Portal>
    <Dialog.Overlay class="overlay" />
    <Dialog.Content class="dlg panel">
      {#if a}
        <Dialog.Title class="ttl">{a.title}</Dialog.Title>
        {#if a.kind === 'confirm'}
          <Dialog.Description class="msg">{a.message}</Dialog.Description>
        {:else}
          <label class="fld">
            {a.label}
            <input
              bind:value
              placeholder={a.placeholder}
              onkeydown={(e) => {
                if (e.key === 'Enter') accept();
              }}
            />
          </label>
        {/if}
        <div class="actions">
          <button onclick={cancel}>{t('dialog.cancel')}</button>
          <button
            class="primary"
            class:danger={a.kind === 'confirm' && a.danger}
            onclick={accept}
          >
            {a.kind === 'confirm' && a.danger ? t('dialog.delete') : t('dialog.ok')}
          </button>
        </div>
      {/if}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  /* Bits portals Overlay + Content to <body> as siblings, so Content centers
     itself rather than relying on the overlay as a grid parent. */
  :global(.overlay) {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 80;
  }
  :global(.dlg) {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 81;
    width: 440px;
    max-width: 92vw;
    padding: 20px;
  }
  :global(.dlg .ttl) {
    margin: 0 0 12px;
    font-size: 14px;
    color: var(--fg);
  }
  :global(.dlg .msg) {
    margin: 0 0 18px;
    color: var(--fg-dim);
    font-size: 13px;
    line-height: 1.5;
  }
  :global(.dlg .fld) {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
    color: var(--fg-dim);
    margin-bottom: 18px;
  }
  :global(.dlg .actions) {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
  }
  :global(.dlg .danger) {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
