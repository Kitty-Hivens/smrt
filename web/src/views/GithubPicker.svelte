<script lang="ts">
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
</script>

<div class="overlay" onclick={onClose} role="presentation">
  <div class="picker panel" onclick={(e) => e.stopPropagation()} role="presentation">
    <div class="hd row">
      <h3>{t('gh.title')}</h3>
      <div class="sp"></div>
      <button onclick={onClose}>{t('common.close')}</button>
    </div>
    <p class="muted hint">{t('gh.hint')}</p>
    <div class="form">
      <Field label={t('gh.repo')}>
        <input class="mono" bind:value={repo} placeholder="Kitty-Hivens/open-smrt-network" />
      </Field>
      <Field label={t('gh.tag')}><input class="mono" bind:value={tag} placeholder="v1.0.0" /></Field>
      <Field label={t('gh.asset')}>
        <input class="mono" bind:value={asset} placeholder="open-smrt-network-1.0.0.jar" />
      </Field>
    </div>
    {#if err}<div class="err mono">{err}</div>{/if}
    <div class="row foot">
      <div class="sp"></div>
      <button class="primary" onclick={add} disabled={!ready || busy}>
        {busy ? t('gh.adding') : t('gh.add')}
      </button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: grid;
    place-items: center;
    z-index: 50;
  }
  .picker {
    width: 520px;
    max-width: 92vw;
    padding: var(--space-4);
  }
  .hd {
    margin-bottom: var(--space-2);
  }
  .hd h3 {
    font-size: 14px;
  }
  .sp {
    flex: 1;
  }
  .hint {
    font-size: 12px;
    margin: 0 0 var(--space-4);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .err {
    color: var(--danger);
    font-size: 12px;
    margin-top: var(--space-3);
  }
  .foot {
    margin-top: var(--space-4);
  }
</style>
