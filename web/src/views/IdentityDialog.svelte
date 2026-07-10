<script lang="ts" module>
  // What the dialog operates on: a cached jar (by sha1), either being assigned
  // fresh (from the needs-identity bucket) or re-edited (from a mod's file row,
  // prefilled with its current mod/release/facets).
  export interface IdentityTarget {
    sha1: string;
    filename: string | null;
    mode: 'assign' | 'edit';
    modId?: number;
    modName?: string;
    version_number?: string;
    channel?: string;
    loaders?: string[];
    mc_versions?: string[];
  }
</script>

<script lang="ts">
  import { api, ApiError, type IdentityInput } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import type { ModSummary } from '../lib/types';

  let {
    target,
    mods,
    onSaved,
    onClose,
  }: {
    target: IdentityTarget;
    mods: ModSummary[];
    onSaved: () => void;
    onClose: () => void;
  } = $props();

  const CHANNELS = ['release', 'beta', 'dev', 'unknown'];

  // svelte-ignore state_referenced_locally -- capture the target's initial values
  // once; the dialog is remounted per open so this is the intended snapshot
  let modMode = $state<'new' | 'existing'>(target.modId != null ? 'existing' : 'new');
  // svelte-ignore state_referenced_locally
  let modName = $state(target.modName ?? '');
  // svelte-ignore state_referenced_locally
  let modId = $state<number | ''>(target.modId ?? '');
  // svelte-ignore state_referenced_locally
  let version = $state(target.version_number ?? '');
  // svelte-ignore state_referenced_locally
  let channel = $state(target.channel ?? 'release');
  // svelte-ignore state_referenced_locally
  let loaders = $state((target.loaders ?? ['forge']).join(', '));
  // svelte-ignore state_referenced_locally
  let mc = $state((target.mc_versions ?? []).join(', '));
  // svelte-ignore state_referenced_locally
  let filename = $state(target.filename ?? '');
  let busy = $state(false);
  let err = $state('');

  const csv = (s: string) =>
    s
      .split(',')
      .map((x) => x.trim())
      .filter(Boolean);

  const canSave = $derived(
    version.trim() !== '' && (modMode === 'new' ? modName.trim() !== '' : modId !== ''),
  );

  async function save() {
    if (!canSave || busy) return;
    busy = true;
    err = '';
    const body: IdentityInput = {
      version_number: version.trim(),
      channel,
      loaders: csv(loaders),
      mc_versions: csv(mc),
      filename: filename.trim() || undefined,
    };
    if (modMode === 'new') body.mod_name = modName.trim();
    else body.mod_id = Number(modId);
    try {
      await api.authorFileIdentity(target.sha1, body);
      onSaved();
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
      busy = false;
    }
  }
</script>

<div class="overlay" onclick={onClose} role="presentation">
  <div class="dlg panel" onclick={(e) => e.stopPropagation()} role="presentation">
    <div class="hd">
      <div class="ttl">{target.mode === 'assign' ? t('id.title') : t('id.titleEdit')}</div>
      <div class="sha faint mono">{target.sha1.slice(0, 16)}</div>
    </div>

    <div class="modmode">
      <button class="seg" class:active={modMode === 'new'} onclick={() => (modMode = 'new')}>
        {t('id.modNew')}
      </button>
      <button
        class="seg"
        class:active={modMode === 'existing'}
        disabled={mods.length === 0}
        onclick={() => (modMode = 'existing')}
      >
        {t('id.modExisting')}
      </button>
    </div>

    {#if modMode === 'new'}
      <label class="fld">
        <span class="lbl">{t('id.modName')}</span>
        <input bind:value={modName} placeholder="JourneyMap" />
      </label>
    {:else}
      <label class="fld">
        <span class="lbl">{t('id.pickMod')}</span>
        <select bind:value={modId}>
          <option value="" disabled>{t('id.pickMod')}</option>
          {#each mods as m (m.mod_id)}
            <option value={m.mod_id}>{m.name}</option>
          {/each}
        </select>
      </label>
    {/if}

    <div class="row2">
      <label class="fld">
        <span class="lbl">{t('id.version')}</span>
        <input bind:value={version} placeholder="1.7.10-5.1.4" />
      </label>
      <label class="fld">
        <span class="lbl">{t('id.channel')}</span>
        <select bind:value={channel}>
          {#each CHANNELS as c}
            <option value={c}>{c}</option>
          {/each}
        </select>
      </label>
    </div>

    <div class="row2">
      <label class="fld">
        <span class="lbl">{t('id.loaders')}</span>
        <input bind:value={loaders} placeholder="forge" />
        <span class="hint faint">{t('id.loadersHint')}</span>
      </label>
      <label class="fld">
        <span class="lbl">{t('id.mc')}</span>
        <input bind:value={mc} placeholder="1.7.10" />
        <span class="hint faint">{t('id.mcHint')}</span>
      </label>
    </div>

    <label class="fld">
      <span class="lbl">{t('id.filename')}</span>
      <input bind:value={filename} placeholder="journeymap-1.7.10-5.1.4.jar" />
    </label>

    {#if err}<div class="err mono">{err}</div>{/if}

    <div class="actions">
      <button onclick={onClose}>{t('dialog.cancel')}</button>
      <button class="primary" disabled={!canSave || busy} onclick={save}>
        {busy ? t('id.saving') : t('id.save')}
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
    z-index: 60;
  }
  .dlg {
    width: 460px;
    max-width: 92vw;
    max-height: 88vh;
    overflow: auto;
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .hd {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .ttl {
    font-size: 15px;
  }
  .sha {
    font-size: 11px;
  }
  .modmode {
    display: flex;
    gap: 2px;
  }
  .seg {
    background: transparent;
    border: 1px solid transparent;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    padding: 5px 12px;
    color: var(--fg-dim);
  }
  .seg.active {
    color: var(--fg);
    border-bottom-color: var(--accent);
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
  }
  .lbl {
    font-size: 12px;
    color: var(--fg-dim);
  }
  .hint {
    font-size: 11px;
  }
  .row2 {
    display: flex;
    gap: var(--space-3);
  }
  .err {
    color: var(--danger);
    font-size: 12px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
</style>
