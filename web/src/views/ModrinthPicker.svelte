<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import type { ModrinthHit, ModrinthVersion } from '../lib/types';

  let {
    mc,
    loader,
    projectType = 'mod',
    onPick,
    onClose,
  }: {
    mc?: string;
    // current pack loader; pre-selects the version filter so the operator sees
    // only compatible versions first
    loader?: string;
    projectType?: string;
    onPick: (sel: { project_id: string; slug: string; version_id: string; title: string }) => void;
    onClose: () => void;
  } = $props();

  let q = $state('');
  let hits = $state<ModrinthHit[]>([]);
  let busy = $state(false);
  let err = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;

  // step 2: version selection for the picked project
  let sel = $state<ModrinthHit | null>(null);
  let versions = $state<ModrinthVersion[]>([]);
  let loadingVers = $state(false);
  let loaderFilter = $state('');
  let releaseOnly = $state(false);

  function onInput() {
    clearTimeout(timer);
    timer = setTimeout(search, 300);
  }

  async function search() {
    if (!q.trim()) {
      hits = [];
      return;
    }
    busy = true;
    err = '';
    try {
      hits = await api.modrinthSearch(q.trim(), mc, projectType);
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      busy = false;
    }
  }

  async function openVersions(h: ModrinthHit) {
    sel = h;
    versions = [];
    err = '';
    loadingVers = true;
    // default the loader filter to the pack's loader when the project ships it
    loaderFilter = '';
    try {
      versions = await api.modrinthVersions(h.slug, mc);
      if (loader && versions.some((v) => v.loaders.includes(loader!))) loaderFilter = loader;
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      loadingVers = false;
    }
  }

  const shownVersions = $derived(
    versions.filter(
      (v) =>
        (!loaderFilter || v.loaders.includes(loaderFilter)) &&
        (!releaseOnly || v.version_type === 'release'),
    ),
  );

  // loaders actually offered by this project's versions, for the filter chips
  const loaderOptions = $derived([...new Set(versions.flatMap((v) => v.loaders))].sort());

  function choose(v: ModrinthVersion) {
    if (!sel) return;
    onPick({ project_id: sel.project_id, slug: sel.slug, version_id: v.id, title: sel.title });
  }

  function back() {
    sel = null;
    versions = [];
    err = '';
  }
</script>

<div class="overlay" onclick={onClose} role="presentation">
  <div class="picker panel" onclick={(e) => e.stopPropagation()} role="presentation">
    {#if !sel}
      <div class="ph row">
        <input bind:value={q} oninput={onInput} placeholder={t('mrp.search')} />
        <button onclick={onClose}>{t('common.close')}</button>
      </div>
      {#if err}<div class="err mono">{err}</div>{/if}
      {#if busy}<div class="muted s">{t('mrp.searching')}</div>{/if}
      <div class="hits scroll">
        {#each hits as h}
          <button class="hit" onclick={() => openVersions(h)}>
            {#if h.icon_url}<img src={h.icon_url} alt="" />{:else}<div class="ic"></div>{/if}
            <div class="info">
              <div class="t">
                {h.title} <span class="faint mono">{h.slug}</span>
                {#if h.author}<span class="faint">· {t('mrp.by', { author: h.author })}</span>{/if}
              </div>
              <div class="d muted">{h.description}</div>
            </div>
          </button>
        {/each}
        {#if hits.length === 0 && q.trim() && !busy}<div class="muted s">{t('mrp.noResults')}</div>{/if}
      </div>
    {:else}
      <div class="ph row">
        <button onclick={back}>{t('mrp.back')}</button>
        <div class="seltitle">
          {sel.title} <span class="faint mono">{sel.slug}</span>
        </div>
        <button onclick={onClose}>{t('common.close')}</button>
      </div>
      <div class="filters">
        <label class="fl">
          {t('mrp.loader')}
          <select bind:value={loaderFilter}>
            <option value="">{t('mrp.anyLoader')}</option>
            {#each loaderOptions as l}<option value={l}>{l}</option>{/each}
          </select>
        </label>
        <label class="fl chk"><input type="checkbox" bind:checked={releaseOnly} /> {t('mrp.releaseOnly')}</label>
        {#if mc}<span class="faint s">{t('mrp.mcLocked', { mc })}</span>{/if}
      </div>
      {#if err}<div class="err mono">{err}</div>{/if}
      {#if loadingVers}<div class="muted s">{t('mrp.loadingVersions')}</div>{/if}
      <div class="hits scroll">
        {#each shownVersions as v (v.id)}
          <button class="vrow" onclick={() => choose(v)}>
            <div class="info">
              <div class="t">
                <span class="vn mono">{v.version_number}</span>
                {#if v.version_type && v.version_type !== 'release'}<span class="chip">{v.version_type}</span>{/if}
              </div>
              <div class="d muted mono">
                {v.loaders.join(', ')}{#if v.game_versions.length} · {v.game_versions.join(', ')}{/if}
              </div>
            </div>
          </button>
        {/each}
        {#if shownVersions.length === 0 && !loadingVers}<div class="muted s">{t('mrp.noVersions')}</div>{/if}
      </div>
    {/if}
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
    width: 620px;
    max-width: 92vw;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    padding: 16px;
  }
  .ph {
    gap: 10px;
    margin-bottom: 10px;
    align-items: center;
  }
  .seltitle {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: 10px;
    flex-wrap: wrap;
  }
  .fl {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--fg-dim);
  }
  .fl select {
    padding: 4px 6px;
    font-size: 12px;
  }
  .chk {
    cursor: pointer;
  }
  .err {
    color: var(--danger);
    font-size: 12px;
    margin-bottom: 8px;
  }
  .s {
    font-size: 12px;
    padding: 6px 0;
  }
  .hits {
    overflow: auto;
  }
  .hit,
  .vrow {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    text-align: left;
    padding: 8px;
    border: 1px solid transparent;
    border-bottom: 1px solid var(--seam);
    background: transparent;
  }
  .hit:hover,
  .vrow:hover {
    background: var(--panel-2);
    border-bottom-color: var(--seam);
  }
  .hit img,
  .hit .ic {
    width: 38px;
    height: 38px;
    object-fit: cover;
    background: var(--bg);
    border: 1px solid var(--seam);
    flex-shrink: 0;
  }
  .info {
    flex: 1;
    min-width: 0;
  }
  .t {
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .vn {
    font-size: 12.5px;
  }
  .chip {
    font-size: 10.5px;
    padding: 1px 6px;
    border: 1px solid var(--seam);
    border-radius: 999px;
    color: var(--warn);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .d {
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  @media (max-width: 560px) {
    .picker {
      padding: var(--space-3);
    }
  }
</style>
