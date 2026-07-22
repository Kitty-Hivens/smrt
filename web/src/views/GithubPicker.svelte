<script lang="ts">
  import { Dialog } from 'bits-ui';
  import { api, ApiError } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import Field from './ui/Field.svelte';

  let {
    onPick,
    onClose,
  }: {
    onPick: (sel: { sha1: string; filename: string }) => void;
    onClose: () => void;
  } = $props();

  let repo = $state('');
  let tag = $state('');
  let asset = $state('');
  let busy = $state(false);
  let err = $state('');

  const ready = $derived(!!(repo.trim() && tag.trim() && asset.trim()));

  async function add() {
    if (!ready || busy) return;
    busy = true;
    err = '';
    try {
      const r = await api.ingestGithub(repo.trim(), tag.trim(), asset.trim());
      onPick({ sha1: r.sha1, filename: asset.trim() });
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
      busy = false;
    }
  }

  // escape / outside-click flip Bits' open to false; the parent unmounts us on close
  function onOpenChange(open: boolean) {
    if (!open) onClose();
  }
</script>

<Dialog.Root open {onOpenChange}>
  <Dialog.Overlay class="dlg-scrim" />
  <Dialog.Content class="ghp-dlg panel">
    <div class="hd row">
      <Dialog.Title level={3} class="ghp-h">{t('gh.title')}</Dialog.Title>
      <div class="sp"></div>
      <button onclick={onClose}>{t('common.close')}</button>
    </div>
    <p class="muted hint">{t('gh.hint')}</p>
    <div class="form">
      <Field label={t('gh.repo')}>
        <input class="mono" bind:value={repo} placeholder="Kitty-Hivens/open-smrt-network" aria-label="Kitty-Hivens/open-smrt-network" />
      </Field>
      <Field label={t('gh.tag')}><input class="mono" bind:value={tag} placeholder="v1.0.0" aria-label="v1.0.0" /></Field>
      <Field label={t('gh.asset')}>
        <input class="mono" bind:value={asset} placeholder="open-smrt-network-1.0.0.jar" aria-label="open-smrt-network-1.0.0.jar" />
      </Field>
    </div>
    {#if err}<div class="err mono">{err}</div>{/if}
    <div class="row foot">
      <div class="sp"></div>
      <button class="primary" onclick={add} disabled={!ready || busy}>
        {busy ? t('gh.adding') : t('gh.add')}
      </button>
    </div>
  </Dialog.Content>
</Dialog.Root>

<style>
  /* Panel + title classes ride on Bits components, so they are global (no scope
     hash) and uniquely named to avoid colliding with the DialogHost .dlg/.overlay
     globals. The backdrop is the shared .dlg-scrim in app.css. */
  :global(.ghp-dlg) {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 61;
    width: 520px;
    max-width: 92vw;
    padding: var(--space-4);
  }
  .hd {
    margin-bottom: var(--space-2);
  }
  :global(.ghp-h) {
    font-size: var(--fs-lg);
  }
  .sp {
    flex: 1;
  }
  .hint {
    font-size: var(--fs-sm);
    margin: 0 0 var(--space-4);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-sm);
    margin-top: var(--space-3);
  }
  .foot {
    margin-top: var(--space-4);
  }
  @media (max-width: 560px) {
    :global(.ghp-dlg) {
      padding: var(--space-3);
    }
  }
</style>
