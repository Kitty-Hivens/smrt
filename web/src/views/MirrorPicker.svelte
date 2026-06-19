<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import type { ModSummary, VersionRow, BuildSummary, BuildModRow, CacheUsageEntry } from '../lib/types';

  let {
    mc,
    loader,
    allowMany = false,
    onPick,
    onAddMany,
    onClose,
  }: {
    mc?: string;
    loader?: string;
    // builds can re-add their whole mod set at once; only offered when adding a
    // new row (not when re-pointing an existing one)
    allowMany?: boolean;
    onPick: (sel: { sha1: string; filename: string }) => void;
    onAddMany?: (items: { sha1: string; filename: string }[]) => void;
    onClose: () => void;
  } = $props();

  type Mode = 'mods' | 'builds' | 'raw';
  let mode = $state<Mode>('mods');
  let err = $state('');

  // ── mods mode ──
  let q = $state('');
  // prefill the facet filters from the pack context; the picker is remounted on
  // each open, so capturing the initial prop value is the intended behaviour
  // svelte-ignore state_referenced_locally
  let loaderF = $state(loader ?? '');
  // svelte-ignore state_referenced_locally
  let mcF = $state(mc ?? '');
  let mods = $state<ModSummary[]>([]);
  let modsLoading = $state(false);
  let modTimer: ReturnType<typeof setTimeout> | undefined;
  // version step for a picked mod
  let selMod = $state<ModSummary | null>(null);
  let modVersions = $state<VersionRow[]>([]);
  let versLoading = $state(false);

  // ── builds mode ──
  let builds = $state<BuildSummary[]>([]);
  let buildsLoading = $state(false);
  let selBuild = $state<BuildSummary | null>(null);
  let buildModRows = $state<BuildModRow[]>([]);
  let bmLoading = $state(false);

  // ── raw cache mode ──
  let raw = $state<CacheUsageEntry[]>([]);
  let rawLoading = $state(false);
  let rawQ = $state('');

  function fail(e: unknown) {
    err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
  }

  async function loadMods() {
    modsLoading = true;
    err = '';
    try {
      mods = await api.registryMods(q.trim() || undefined, loaderF.trim() || undefined, mcF.trim() || undefined);
    } catch (e) {
      fail(e);
    } finally {
      modsLoading = false;
    }
  }
  function onModFilter() {
    clearTimeout(modTimer);
    modTimer = setTimeout(loadMods, 250);
  }

  async function openMod(m: ModSummary) {
    selMod = m;
    modVersions = [];
    err = '';
    versLoading = true;
    try {
      modVersions = await api.registryModVersions(m.mod_id);
    } catch (e) {
      fail(e);
    } finally {
      versLoading = false;
    }
  }

  function pickVersion(v: VersionRow) {
    onPick({ sha1: v.sha1, filename: v.filename || `${selMod?.name ?? v.sha1.slice(0, 12)}.jar` });
  }

  async function loadBuilds() {
    buildsLoading = true;
    err = '';
    try {
      builds = await api.registryBuilds();
    } catch (e) {
      fail(e);
    } finally {
      buildsLoading = false;
    }
  }

  async function openBuild(b: BuildSummary) {
    selBuild = b;
    buildModRows = [];
    err = '';
    bmLoading = true;
    try {
      buildModRows = await api.registryBuildMods(b.pack_id, b.pack_version);
    } catch (e) {
      fail(e);
    } finally {
      bmLoading = false;
    }
  }

  function addAllFromBuild() {
    if (!onAddMany) return;
    onAddMany(buildModRows.map((m) => ({ sha1: m.sha1, filename: m.filename })));
  }

  async function loadRaw() {
    rawLoading = true;
    err = '';
    try {
      raw = (await api.cacheUsage()).entries;
    } catch (e) {
      fail(e);
    } finally {
      rawLoading = false;
    }
  }

  const rawShown = $derived(
    raw.filter((e) => {
      const n = rawQ.trim().toLowerCase();
      if (!n) return true;
      return e.sha1.includes(n) || (e.uses[0]?.filename ?? '').toLowerCase().includes(n);
    }),
  );
  const rawName = (e: CacheUsageEntry) => e.uses[0]?.filename ?? '';

  function setMode(m: Mode) {
    mode = m;
    selMod = null;
    selBuild = null;
    err = '';
    if (m === 'mods' && mods.length === 0) loadMods();
    if (m === 'builds' && builds.length === 0) loadBuilds();
    if (m === 'raw' && raw.length === 0) loadRaw();
  }

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    const u = ['KB', 'MB', 'GB'];
    let i = -1;
    do {
      n /= 1024;
      i++;
    } while (n >= 1024 && i < u.length - 1);
    return `${n.toFixed(1)} ${u[i]}`;
  }

  // initial load
  loadMods();
</script>

<div class="overlay" onclick={onClose} role="presentation">
  <div class="picker panel" onclick={(e) => e.stopPropagation()} role="presentation">
    <div class="ph row">
      <div class="tabs">
        <button class="seg" class:active={mode === 'mods'} onclick={() => setMode('mods')}>{t('mirror.tab.mods')}</button>
        <button class="seg" class:active={mode === 'builds'} onclick={() => setMode('builds')}>{t('mirror.tab.builds')}</button>
        <button class="seg" class:active={mode === 'raw'} onclick={() => setMode('raw')}>{t('mirror.tab.raw')}</button>
      </div>
      <div class="spacer"></div>
      <button onclick={onClose}>{t('common.close')}</button>
    </div>

    {#if err}<div class="err mono">{err}</div>{/if}

    {#if mode === 'mods'}
      {#if !selMod}
        <div class="filters">
          <input class="grow" bind:value={q} oninput={onModFilter} placeholder={t('mirror.search')} />
          <input class="sm" bind:value={loaderF} oninput={onModFilter} placeholder={t('mirror.loader')} />
          <input class="sm" bind:value={mcF} oninput={onModFilter} placeholder={t('mirror.mc')} />
        </div>
        {#if modsLoading}<div class="muted s">{t('common.loading')}</div>{/if}
        <div class="hits scroll">
          {#each mods as m (m.mod_id)}
            <button class="hit" onclick={() => openMod(m)}>
              <div class="info">
                <div class="t">
                  {m.name}
                  {#if m.author}<span class="faint">· {t('mirror.by', { author: m.author })}</span>{/if}
                </div>
                <div class="d muted mono">
                  {#if m.loaders.length}{m.loaders.join(', ')}{/if}
                  {#if m.mc_versions.length} · {m.mc_versions.join(', ')}{/if}
                </div>
              </div>
              <span class="cnt faint mono">{t('mirror.versionsN', { n: m.version_count })}</span>
            </button>
          {/each}
          {#if mods.length === 0 && !modsLoading}<div class="muted s">{t('mirror.noMods')}</div>{/if}
        </div>
      {:else}
        <div class="ph row">
          <button onclick={() => (selMod = null)}>{t('mrp.back')}</button>
          <div class="seltitle">{selMod.name}</div>
        </div>
        {#if versLoading}<div class="muted s">{t('common.loading')}</div>{/if}
        <div class="hits scroll">
          {#each modVersions as v (v.sha1)}
            <button class="vrow" onclick={() => pickVersion(v)}>
              <div class="info">
                <div class="t"><span class="mono">{v.version}</span></div>
                <div class="d muted mono">
                  {v.targets.join(', ')}{#if v.mc_versions.length} · {v.mc_versions.join(', ')}{/if} · {fmtBytes(v.size_bytes)}
                </div>
              </div>
              <span class="cnt faint mono">{v.sha1.slice(0, 10)}</span>
            </button>
          {/each}
          {#if modVersions.length === 0 && !versLoading}<div class="muted s">{t('mirror.noVersions')}</div>{/if}
        </div>
      {/if}
    {:else if mode === 'builds'}
      {#if !selBuild}
        {#if buildsLoading}<div class="muted s">{t('common.loading')}</div>{/if}
        <div class="hits scroll">
          {#each builds as b (b.pack_id + b.pack_version)}
            <button class="hit" onclick={() => openBuild(b)}>
              <div class="info">
                <div class="t">
                  {b.pack_id} <span class="faint mono">{b.pack_version}</span>
                  {#if b.is_latest}<span class="chip ok">{t('mirror.latest')}</span>{/if}
                </div>
                <div class="d muted mono">
                  {b.mc_version}{#if b.loader_id} · {b.loader_id}{#if b.loader_version} {b.loader_version}{/if}{/if}
                </div>
              </div>
              <span class="cnt faint mono">{t('mirror.modsN', { n: b.mod_count })}</span>
            </button>
          {/each}
          {#if builds.length === 0 && !buildsLoading}<div class="muted s">{t('mirror.noBuilds')}</div>{/if}
        </div>
      {:else}
        <div class="ph row">
          <button onclick={() => (selBuild = null)}>{t('mrp.back')}</button>
          <div class="seltitle">{selBuild.pack_id} <span class="faint mono">{selBuild.pack_version}</span></div>
          {#if allowMany && onAddMany && buildModRows.length}
            <button class="primary sm" onclick={addAllFromBuild}>{t('mirror.addAll', { n: buildModRows.length })}</button>
          {/if}
        </div>
        {#if bmLoading}<div class="muted s">{t('common.loading')}</div>{/if}
        <div class="hits scroll">
          {#each buildModRows as m (m.sha1 + m.filename)}
            <div class="vrow static">
              <div class="info">
                <div class="t">
                  {m.name} <span class="faint mono">{m.version}</span>
                  {#if !m.required}<span class="chip">{t('mr.optional')}</span>{/if}
                </div>
                <div class="d muted mono">
                  {m.filename}{#if m.targets.length} · {m.targets.join(', ')}{/if}
                </div>
              </div>
              <button class="sm" onclick={() => onPick({ sha1: m.sha1, filename: m.filename })}>{t('mirror.add')}</button>
            </div>
          {/each}
          {#if buildModRows.length === 0 && !bmLoading}<div class="muted s">{t('mirror.noBuildMods')}</div>{/if}
        </div>
      {/if}
    {:else}
      <div class="filters">
        <input class="grow" bind:value={rawQ} placeholder={t('cachePick.search')} />
      </div>
      {#if rawLoading}<div class="muted s">{t('common.loading')}</div>{/if}
      <div class="hits scroll">
        {#each rawShown as e (e.sha1)}
          <button class="hit" onclick={() => onPick({ sha1: e.sha1, filename: rawName(e) || `${e.sha1.slice(0, 12)}.jar` })}>
            <div class="info">
              <div class="t">
                {rawName(e) || t('cachePick.noName')}
                {#if e.uses.length === 0}<span class="chip warn">{t('cachePick.orphan')}</span>{/if}
              </div>
              <div class="d muted mono">{e.sha1.slice(0, 16)} · {fmtBytes(e.size_bytes)}</div>
            </div>
          </button>
        {/each}
        {#if rawShown.length === 0 && !rawLoading}
          <div class="muted s">{rawQ.trim() ? t('cachePick.noMatch') : t('cachePick.empty')}</div>
        {/if}
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
    width: 660px;
    max-width: 92vw;
    max-height: 82vh;
    display: flex;
    flex-direction: column;
    padding: var(--space-4);
  }
  .ph {
    gap: var(--space-2);
    margin-bottom: var(--space-2);
    align-items: center;
  }
  .tabs {
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
  .spacer {
    flex: 1;
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
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }
  .filters .grow {
    flex: 1;
  }
  .filters .sm {
    width: 110px;
  }
  .err {
    color: var(--danger);
    font-size: 12px;
    margin-bottom: var(--space-2);
  }
  .s {
    font-size: 12px;
    padding: var(--space-2) 0;
  }
  .hits {
    overflow: auto;
  }
  .hit,
  .vrow {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    text-align: left;
    padding: var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    border-bottom: 1px solid var(--seam);
    background: transparent;
  }
  .hit:hover,
  .vrow:not(.static):hover {
    background: var(--panel-2);
  }
  .info {
    flex: 1;
    min-width: 0;
  }
  .t {
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .d {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin-top: 2px;
  }
  .cnt {
    font-size: 11px;
    flex-shrink: 0;
  }
  .chip {
    font-size: 10.5px;
    padding: 1px 6px;
    border: 1px solid var(--seam);
    border-radius: 999px;
    color: var(--fg-dim);
  }
  .chip.ok {
    color: var(--ok);
    border-color: color-mix(in srgb, var(--ok) 45%, transparent);
  }
  .chip.warn {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 45%, transparent);
  }
  button.sm {
    padding: 4px 10px;
    font-size: 12px;
    flex-shrink: 0;
  }
</style>
