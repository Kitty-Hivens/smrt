<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { dialogs } from '../lib/dialogs.svelte';
  import { t } from '../lib/i18n.svelte';
  import { isDebug } from '../lib/roles';
  import type { JarDiff, ModSummary, ReleaseRow, UnassignedJar, VersionRow } from '../lib/types';
  import ModIcon from './ModIcon.svelte';
  import IdentityDialog, { type IdentityTarget } from './IdentityDialog.svelte';
  import DropZone from './ui/DropZone.svelte';

  // Compat-affecting authoring -- assigning/editing a jar's identity or a
  // release's version -- is debug-gated on the server (#39). Hide those controls
  // for a plain admin so they never click a button that would 403.
  let canDebug = $state(false);
  api
    .me()
    .then((m) => (canDebug = isDebug(m?.role)))
    .catch(() => {});

  let mods = $state<ModSummary[]>([]);
  let unassigned = $state<UnassignedJar[]>([]);
  let removed = $state<string[]>([]);
  let err = $state('');
  let loading = $state(true);
  let q = $state('');

  // the expanded mod and its lazily-loaded releases
  let openId = $state<number | null>(null);
  let releases = $state<ReleaseRow[]>([]);
  let relLoading = $state(false);

  // a file whose sha1 Modrinth confirmed is authentic; a self-hosted file under a
  // mod that ALSO has a Modrinth-verified one is a likely repackage (the SC case)
  const modHasVerified = $derived(
    releases.some((r) => r.files.some((f) => f.modrinth_version_id)),
  );

  let idTarget = $state<IdentityTarget | null>(null);

  let uploading = $state(false);
  let upMsg = $state('');

  const fail = (e: unknown) => (e instanceof ApiError ? `${e.status} ${e.body}` : String(e));

  async function load() {
    loading = true;
    err = '';
    try {
      const [m, u, rm] = await Promise.all([
        api.registryMods(q.trim() || undefined),
        api.unassigned(),
        api.removed(),
      ]);
      mods = m;
      unassigned = u;
      removed = rm.removed;
    } catch (e) {
      err = fail(e);
    } finally {
      loading = false;
    }
  }
  load();

  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  function onSearch() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(load, 250);
  }

  async function toggle(m: ModSummary) {
    if (openId === m.mod_id) {
      openId = null;
      releases = [];
      return;
    }
    openId = m.mod_id;
    releases = [];
    relLoading = true;
    try {
      releases = await api.modReleases(m.mod_id);
    } catch (e) {
      err = fail(e);
    } finally {
      relLoading = false;
    }
  }

  async function reloadOpen() {
    if (openId == null) return;
    try {
      releases = await api.modReleases(openId);
    } catch (e) {
      err = fail(e);
    }
  }

  async function onDropJars(files: File[]) {
    uploading = true;
    upMsg = '';
    let n = 0;
    try {
      for (const f of files) {
        if (!f.name.toLowerCase().endsWith('.jar')) continue;
        await api.uploadCacheJar(f);
        n++;
      }
      upMsg = t('mm.uploaded', { count: n });
    } catch (x) {
      upMsg = fail(x);
    } finally {
      await load();
      uploading = false;
    }
  }

  function assign(u: UnassignedJar) {
    idTarget = { sha1: u.sha1, filename: null, mode: 'assign' };
  }

  function editFile(f: VersionRow, rel: ReleaseRow, modName: string) {
    idTarget = {
      sha1: f.sha1,
      filename: f.filename ?? null,
      mode: 'edit',
      modId: openId ?? undefined,
      modName,
      version_number: rel.version_number,
      channel: rel.channel,
      loaders: f.targets.filter((x) => x !== 'any'),
      mc_versions: f.mc_versions,
    };
  }

  async function onSaved() {
    idTarget = null;
    await load();
    await reloadOpen();
  }

  async function rename(m: ModSummary, e: Event) {
    e.stopPropagation();
    const name = (
      await dialogs.prompt(t('mm.renamePrompt'), { title: t('mm.renameTitle'), initial: m.name })
    )?.trim();
    if (!name) return;
    try {
      await api.renameMod(m.mod_id, { name });
      await load();
    } catch (x) {
      err = fail(x);
    }
  }

  // Merge this mod into another (the target survives). Debug-only registry
  // surgery: the operator gives the surviving mod's id (shown as #id on each row).
  async function merge(m: ModSummary, e: Event) {
    e.stopPropagation();
    const raw = await dialogs.prompt(t('mm.mergePrompt', { name: m.name, id: m.mod_id }), {
      title: t('mm.mergeTitle'),
    });
    if (raw == null) return;
    const into = parseInt(raw.trim(), 10);
    if (!Number.isFinite(into) || into === m.mod_id) {
      err = t('mm.mergeBadId');
      return;
    }
    const ok = await dialogs.confirm(t('mm.mergeConfirm', { from: m.mod_id, into }), {
      danger: true,
    });
    if (!ok) return;
    try {
      await api.mergeMods(m.mod_id, into);
      if (openId === m.mod_id) openId = null;
      await load();
    } catch (x) {
      err = fail(x);
    }
  }

  // Repackage (tamper) diff: for a self-hosted file under a mod that also has a
  // Modrinth-verified sibling, show what it changed vs the genuine build. Toggles
  // an inline panel; the changed classes are the signal, resources are noise.
  let diffFor = $state<string | null>(null);
  let diffData = $state<JarDiff | null>(null);
  let diffLoading = $state(false);
  let diffErr = $state('');

  async function showDiff(f: VersionRow) {
    if (diffFor === f.sha1) {
      diffFor = null;
      diffData = null;
      return;
    }
    diffFor = f.sha1;
    diffData = null;
    diffErr = '';
    diffLoading = true;
    try {
      diffData = await api.repackDiff(f.sha1);
    } catch (e) {
      diffErr = fail(e);
    } finally {
      diffLoading = false;
    }
  }

  async function editReleaseVersion(rel: ReleaseRow) {
    const v = (
      await dialogs.prompt(t('mm.versionPrompt'), {
        title: t('mm.editReleaseTitle'),
        initial: rel.version_number,
      })
    )?.trim();
    if (!v || v === rel.version_number) return;
    try {
      await api.editRelease(rel.release_id, { version_number: v });
      await reloadOpen();
    } catch (x) {
      err = fail(x);
    }
  }

  async function delFile(f: VersionRow) {
    const name = f.filename || f.sha1.slice(0, 12);
    const ok = await dialogs.confirm(t('cache.deleteMsg', { name }), {
      title: t('cache.deleteTitle'),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteCacheJar(f.sha1);
      await load();
      await reloadOpen();
    } catch (x) {
      err = fail(x);
    }
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

  // Wide mods span many MC versions; the backend returns them oldest-first, so a
  // long run collapses to its bounds plus a count rather than a flat tag soup.
  function mcFacet(vs: string[]): { span: boolean; items: string[]; count: number } {
    if (vs.length <= 4) return { span: false, items: vs, count: vs.length };
    return { span: true, items: [vs[0], vs[vs.length - 1]], count: vs.length };
  }
</script>

<div class="view">
  {#if err}<div class="err mono">{err}</div>{/if}

  <DropZone
    accept=".jar"
    label={uploading ? t('mm.uploading') : t('mm.drop')}
    busy={uploading}
    onFiles={onDropJars}
  />
  {#if upMsg}<div class="upmsg muted mono">{upMsg}</div>{/if}

  {#if unassigned.length}
    <section class="panel bucket">
      <div class="bhead">
        <span class="btitle">{t('mm.needsIdentity')}</span>
        <span class="faint">{t('mm.needsIdentitySub', { n: unassigned.length })}</span>
      </div>
      {#each unassigned as u (u.sha1)}
        <div class="urow">
          <div class="uinfo">
            <span class="mono">{u.sha1.slice(0, 16)}</span>
            <span class="faint mono">{fmtBytes(u.size_bytes)}</span>
          </div>
          {#if canDebug}
            <button class="primary sm" onclick={() => assign(u)}>{t('mm.assign')}</button>
          {/if}
        </div>
      {/each}
    </section>
  {/if}

  <input class="search" bind:value={q} oninput={onSearch} placeholder={t('mm.search')} />

  <div class="panel modlist">
    {#each mods as m (m.mod_id)}
      <div class="mod" class:open={openId === m.mod_id}>
        <div
          class="modrow"
          role="button"
          tabindex="0"
          aria-expanded={openId === m.mod_id}
          onclick={() => toggle(m)}
          onkeydown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              toggle(m);
            }
          }}
        >
          <span class="chev" aria-hidden="true">&#9656;</span>
          <ModIcon name={m.name} source={{ type: 'smrt_static', rel_path: '' }} size={32} mono />
          <div class="minfo">
            <div class="mname">
              {m.name}{#if m.author}<span class="mby">{t('mm.by', { author: m.author })}</span>{/if}
            </div>
            {#if m.loaders.length || m.mc_versions.length}
              <div class="mtags">
                {#if m.loaders.length}
                  <span class="facet">
                    {#each m.loaders as l}<span class="tag">{l}</span>{/each}
                  </span>
                {/if}
                {#if m.mc_versions.length}
                  {@const mc = mcFacet(m.mc_versions)}
                  <span class="facet mc">
                    {#if mc.span}
                      <span class="tag">{mc.items[0]}</span>
                      <span class="ell" aria-hidden="true">&hellip;</span>
                      <span class="tag">{mc.items[1]}</span>
                      <span class="fcount mono">{mc.count}</span>
                    {:else}
                      {#each mc.items as v}<span class="tag">{v}</span>{/each}
                    {/if}
                  </span>
                {/if}
              </div>
            {/if}
          </div>
          <span class="cnt mono">{t('mirror.versionsN', { n: m.version_count })}</span>
        </div>

        {#if openId === m.mod_id}
          <div class="rels">
            <div class="modactions">
              <button class="link" onclick={(e) => rename(m, e)} title={t('mm.renameTitle')}>
                {t('mm.rename')}
              </button>
              {#if canDebug}
                <button class="link" onclick={(e) => merge(m, e)} title={t('mm.mergeTitle')}>
                  {t('mm.merge')}
                </button>
                <span class="faint mono modid">#{m.mod_id}</span>
              {/if}
            </div>
            {#if relLoading}
              <div class="muted s">{t('common.loading')}</div>
            {/if}
            {#each releases as rel (rel.release_id)}
              <div class="rel">
                <div class="relhead">
                  <span class="rver mono">{rel.version_number}</span>
                  <span class="chip ch-{rel.channel}">{rel.channel}</span>
                  <span class="faint mono">{t('mm.filesN', { n: rel.files.length })}</span>
                  {#if canDebug}
                    <button class="link sm" onclick={() => editReleaseVersion(rel)}>{t('mm.edit')}</button>
                  {/if}
                </div>
                {#each rel.files as f (f.sha1)}
                  <div class="file">
                    <ModIcon
                      name={f.filename ?? m.name}
                      source={{ type: 'smrt_cache', sha1: f.sha1 }}
                      size={22}
                      mono
                    />
                    <div class="finfo">
                      <div class="fname">{f.filename ?? f.sha1.slice(0, 16)}</div>
                      <div class="fmeta muted mono">
                        {f.targets.join(', ')}{#if f.mc_versions.length} · {f.mc_versions.join(', ')}{/if}
                        · {fmtBytes(f.size_bytes)}{#if !f.cached} · {t('mm.uncached')}{/if}
                      </div>
                    </div>
                    {#if f.modrinth_version_id}
                      <span class="chip verified" title="Modrinth-verified">{t('mm.verified')}</span>
                    {:else if modHasVerified}
                      <span class="chip repack" title={t('mm.repackHint')}>{t('mm.repack')}</span>
                    {:else}
                      <span class="chip">{t('mm.selfhost')}</span>
                    {/if}
                    <div class="factions">
                      {#if !f.modrinth_version_id && modHasVerified && f.cached}
                        <button
                          class="link"
                          class:active={diffFor === f.sha1}
                          onclick={() => showDiff(f)}>{t('mm.diff')}</button>
                      {/if}
                      {#if canDebug}
                        <button class="link" onclick={() => editFile(f, rel, m.name)}>{t('mm.edit')}</button>
                      {/if}
                      <button class="link danger" onclick={() => delFile(f)}>{t('common.delete')}</button>
                    </div>
                  </div>
                  {#if diffFor === f.sha1}
                    <div class="diffpanel">
                      {#if diffLoading}
                        <div class="muted s">{t('common.loading')}</div>
                      {:else if diffErr}
                        <div class="err mono">{diffErr}</div>
                      {:else if diffData}
                        <div class="diffsum mono">
                          {t('mm.diffClasses', { n: diffData.changed_classes.length })} ·
                          {t('mm.diffResources', { n: diffData.changed_resources.length })} ·
                          {t('mm.diffAdded', { n: diffData.added.length })} ·
                          {t('mm.diffRemoved', { n: diffData.removed.length })} ·
                          {t('mm.diffIdentical', { n: diffData.identical })}
                        </div>
                        {#if diffData.changed_classes.length}
                          <div class="diffh">{t('mm.diffClassesH')}</div>
                          {#each diffData.changed_classes as c}
                            <div class="mono diffrow">{c}</div>
                          {/each}
                        {:else}
                          <div class="muted s">{t('mm.diffNoClasses')}</div>
                        {/if}
                      {/if}
                    </div>
                  {/if}
                {/each}
              </div>
            {/each}
            {#if !relLoading && releases.length === 0}
              <div class="muted s">{t('mirror.noVersions')}</div>
            {/if}
          </div>
        {/if}
      </div>
    {/each}
    {#if mods.length === 0 && !loading}
      <div class="muted empty">{t('mm.noMods')}</div>
    {/if}
  </div>

  {#if removed.length}
    <h2 class="sec">{t('cache.removedTitle')}</h2>
    <div class="cache-head muted">{t('cache.removedSub', { count: removed.length })}</div>
    <div class="panel">
      {#each removed as sha}
        <div class="rmrow mono faint">{sha}</div>
      {/each}
    </div>
  {/if}

  {#if idTarget}
    {#key idTarget.sha1}
      <IdentityDialog target={idTarget} {mods} {onSaved} onClose={() => (idTarget = null)} />
    {/key}
  {/if}
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .err {
    color: var(--danger);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: var(--radius-sm);
    padding: var(--space-3) var(--space-4);
    font-size: 12px;
  }
  .upmsg {
    font-size: 12px;
    margin-top: -8px;
  }
  .bucket {
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 4px;
    border-color: color-mix(in srgb, var(--warn) 35%, var(--seam));
  }
  .bhead {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    margin-bottom: 4px;
  }
  .btitle {
    font-size: 13px;
    color: var(--warn);
  }
  .urow {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 5px 4px;
    border-top: 1px solid var(--seam);
  }
  .uinfo {
    flex: 1;
    display: flex;
    gap: var(--space-3);
    font-size: 12px;
  }
  .search {
    max-width: 420px;
  }
  .modlist {
    overflow: hidden;
  }
  .mod {
    border-bottom: 1px solid var(--seam);
  }
  .mod:last-child {
    border-bottom: none;
  }
  .modrow {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-radius: 0;
    padding: var(--space-3);
    cursor: pointer;
  }
  .modrow:hover {
    background: var(--panel-2);
  }
  .chev {
    color: var(--fg-faint);
    font-size: 11px;
    flex: none;
    transition: transform 0.15s ease;
  }
  .mod.open .chev {
    transform: rotate(90deg);
    color: var(--fg-dim);
  }
  .minfo {
    flex: 1;
    min-width: 0;
  }
  .mname {
    font-size: 14px;
    font-weight: 600;
  }
  .mby {
    color: var(--fg-faint);
    font-size: 12px;
    font-weight: 400;
    margin-left: 6px;
  }
  .mtags {
    display: flex;
    gap: 5px;
    margin-top: 6px;
    flex-wrap: wrap;
    align-items: center;
  }
  .facet {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex-wrap: wrap;
  }
  .facet.mc {
    padding-left: 8px;
    margin-left: 3px;
    border-left: 1px solid var(--seam);
  }
  .ell {
    color: var(--fg-faint);
    font-size: 11px;
  }
  .fcount {
    font-size: 10px;
    color: var(--fg-faint);
    margin-left: 1px;
  }
  .modactions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: 0 0 4px;
  }
  .modid {
    font-size: 11px;
  }
  .factions {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }
  .link.active {
    color: var(--accent-strong);
  }
  .diffpanel {
    margin: 2px 0 var(--space-3) 34px;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--seam);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .diffsum {
    font-size: 11px;
    color: var(--fg-dim);
    margin-bottom: 6px;
  }
  .diffh {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--warn);
    margin: 4px 0;
  }
  .diffrow {
    font-size: 11px;
    padding: 1px 0;
    overflow-wrap: anywhere;
  }
  .cnt {
    font-size: 11px;
    color: var(--fg-faint);
    flex-shrink: 0;
  }
  .rels {
    padding: 2px 0 var(--space-3) 42px;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .rel {
    border-left: 2px solid var(--seam-bright);
    padding-left: var(--space-3);
  }
  .relhead {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px 0;
  }
  .rver {
    font-size: 12.5px;
  }
  .file {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 3px 0;
  }
  .finfo {
    flex: 1;
    min-width: 0;
  }
  .fname {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fmeta {
    font-size: 10.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip {
    font-size: 10px;
    padding: 1px 7px;
    border: 1px solid var(--seam);
    border-radius: 999px;
    color: var(--fg-dim);
    flex-shrink: 0;
  }
  .chip.verified {
    color: var(--info);
    border-color: color-mix(in srgb, var(--info) 45%, var(--seam));
    background: var(--info-soft);
  }
  .chip.repack {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 45%, var(--seam));
    background: var(--warn-soft);
  }
  .link {
    background: transparent;
    border: none;
    border-radius: 0;
    color: var(--fg-dim);
    padding: 2px 6px;
    font-size: 11px;
    flex-shrink: 0;
  }
  .link:hover {
    color: var(--fg);
  }
  .link.danger:hover {
    color: var(--danger);
  }
  button.sm {
    padding: 4px 10px;
    font-size: 12px;
    flex-shrink: 0;
  }
  .empty,
  .s {
    padding: var(--space-3);
    font-size: 12px;
  }
  .sec {
    font-size: 13px;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: var(--space-3) 0 6px;
  }
  .cache-head {
    font-size: 12px;
    margin-bottom: 8px;
  }
  .rmrow {
    padding: 4px var(--space-3);
    font-size: 11px;
    border-bottom: 1px solid var(--seam);
  }
</style>
