<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { dialogs } from '../lib/dialogs.svelte';
  import { route } from '../lib/route.svelte';
  import { reload } from '../lib/reload.svelte';
  import { t } from '../lib/i18n.svelte';
  import type { ModSummary, PackSummary, ServerEntry, UnassignedJar } from '../lib/types';
  import ServerEditor from './ServerEditor.svelte';
  import PackEditor from './PackEditor.svelte';
  import ModManager from './ModManager.svelte';
  import UsersView from './UsersView.svelte';

  // the active section comes from the shared route store; the shell rail drives it

  // server create/edit: 'new' = creating, ServerEntry = editing, null = closed
  let serverEdit = $state<ServerEntry | 'new' | null>(null);
  // pack editor: pack_id being edited, null = closed
  let packEdit = $state<string | null>(null);

  let packs = $state<PackSummary[]>([]);
  let servers = $state<ServerEntry[]>([]);
  let mods = $state<ModSummary[]>([]);
  let unassigned = $state<UnassignedJar[]>([]);
  let removed = $state<string[]>([]);
  let authoring = $state<string[]>([]);
  let cacheBytes = $state(0);
  let cacheCount = $state(0);
  let err = $state('');
  let loading = $state(true);

  async function loadAll() {
    loading = true;
    reload.setBusy(true);
    err = '';
    try {
      const [p, s, md, u, a, rm, ci] = await Promise.all([
        api.adminSummaries(),
        api.servers(),
        api.registryMods(),
        api.unassigned(),
        api.authoringPacks(),
        api.removed(),
        api.cacheInventory(),
      ]);
      packs = p;
      servers = s.servers;
      mods = md;
      unassigned = u;
      authoring = a.packs;
      removed = rm.removed;
      cacheBytes = ci.entries.reduce((n, e) => n + e.size_bytes, 0);
      cacheCount = ci.entries.length;
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      loading = false;
      reload.setBusy(false);
    }
  }
  loadAll();
  // the shell's top-bar refresh bumps reload.count; reload when it does
  $effect(() => {
    if (reload.count > 0) loadAll();
  });

  async function delServer(id: string) {
    const ok = await dialogs.confirm(t('servers.deleteMsg', { id }), {
      title: t('servers.deleteTitle'),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteServer(id);
      await loadAll();
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
  }

  // publish/unpublish a built pack: flips visibility between draft and published.
  // Takes effect on the public listing immediately (the mirror patches summary.json).
  async function togglePublish(p: PackSummary) {
    const next = p.visibility === 'published' ? 'draft' : 'published';
    try {
      await api.setVisibility(p.pack_id, next);
      await loadAll();
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
  }

  // typed lookup so t() gets a literal MsgKey, not a dynamic string
  const visKey = {
    draft: 'packs.vis.draft',
    unlisted: 'packs.vis.unlisted',
    published: 'packs.vis.published',
  } as const;

  async function delPack(id: string) {
    const ok = await dialogs.confirm(t('packs.deleteMsg', { id }), {
      title: t('packs.deleteTitle'),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deletePack(id);
      if (packEdit === id) packEdit = null;
      await loadAll();
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
  }

  const authoringSet = $derived(new Set(authoring));
  const allPackIds = $derived(
    [...new Set([...packs.map((p) => p.pack_id), ...authoring])].sort(),
  );
  const summaryFor = (id: string) => packs.find((p) => p.pack_id === id);

  // overview metrics
  const unbuiltCount = $derived(allPackIds.filter((id) => !summaryFor(id)).length);
  const builtCount = $derived(allPackIds.length - unbuiltCount);
  const featPackCount = $derived(packs.filter((p) => p.featured).length);
  const featServerCount = $derived(servers.filter((s) => s.featured).length);

  // recent builds: built packs, newest first by the date baked into the version
  // slug (SNAPSHOT-<ver>-<YYYY.MM.DD>). No separate build log to read.
  const recentBuilds = $derived(
    packs
      .filter((p) => p.latest_pack_version)
      .map((p) => ({
        pack: p.display_name,
        ver: p.latest_pack_version,
        date: p.latest_pack_version.match(/(\d{4}\.\d{2}\.\d{2})/)?.[1] ?? '',
      }))
      .sort((a, b) => b.date.localeCompare(a.date)),
  );
  const cacheNum = $derived(
    cacheBytes >= 1e9
      ? (cacheBytes / 1e9).toFixed(1)
      : cacheBytes >= 1e6
        ? (cacheBytes / 1e6).toFixed(0)
        : `${Math.max(1, Math.round(cacheBytes / 1e3))}`,
  );
  const cacheUnit = $derived(cacheBytes >= 1e9 ? 'GB' : cacheBytes >= 1e6 ? 'MB' : 'KB');

  async function newPack() {
    const id = (
      await dialogs.prompt(t('packs.newPrompt'), { title: t('packs.new') })
    )?.trim();
    if (id) packEdit = id;
  }
</script>

<div class="view">
  {#if err}<div class="err mono">{err}</div>{/if}

  <div class="body">
    {#if route.section === 'overview'}
      <section class="ov">
        <div class="seclabel">{t('overview.status')}</div>
        <div class="readout">
          <div class="stat">
            <div class="k">{t('overview.packs')}</div>
            <div class="v">{allPackIds.length}</div>
            <div class="s">{t('overview.packsSub', { built: builtCount, unbuilt: unbuiltCount })}</div>
          </div>
          <div class="stat">
            <div class="k">{t('mm.overviewMods')}</div>
            <div class="v">{mods.length}</div>
            <div class="s">
              {unassigned.length
                ? t('mm.overviewModsSub', { n: unassigned.length })
                : `${t('overview.authoring')}: ${authoring.length}`}
            </div>
          </div>
          <div class="stat">
            <div class="k">{t('overview.cache')}</div>
            <div class="v">{cacheNum}<small class="unit">{cacheUnit}</small></div>
            <div class="s">{cacheCount} jar</div>
          </div>
          <div class="stat">
            <div class="k">{#if removed.length}<span class="d"></span>{/if}{t('overview.takedown')}</div>
            <div class="v">{removed.length}</div>
            <div class="s">blocked sha1</div>
          </div>
        </div>

        <div class="cols">
          <div class="card">
            <h3>{t('overview.packs')}</h3>
            {#each allPackIds as id}
              {@const p = summaryFor(id)}
              <div
                class="lrow clickable"
                role="button"
                tabindex="0"
                onclick={() => {
                  route.go('packs');
                  packEdit = id;
                }}
                onkeydown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    route.go('packs');
                    packEdit = id;
                  }
                }}
              >
                <span class="av">{(p?.display_name ?? id).slice(0, 2).toUpperCase()}</span>
                <div class="lcol">
                  <div class="nm">{p?.display_name ?? id}</div>
                  <div class="mm">
                    {p?.minecraft_version ?? '-'} &middot; {p?.latest_pack_version ?? t('packs.unbuilt')}
                  </div>
                </div>
                <div class="grow"></div>
                {#if p}
                  <span class="chip ok"><span class="g"></span>built</span>
                {:else}
                  <span class="chip"><span class="g"></span>{t('packs.unbuilt')}</span>
                {/if}
              </div>
            {/each}
            {#if allPackIds.length === 0 && !loading}
              <div class="lrow"><span class="muted">{t('packs.empty')}</span></div>
            {/if}
          </div>

          <div class="card">
            <h3>{t('overview.recent')}</h3>
            {#each recentBuilds as b}
              <div class="frow">
                <span class="ft">{b.date || '—'}</span>
                <span class="fx"><b>{b.pack}</b> {t('overview.built')} &middot; <span class="mono">{b.ver}</span></span>
              </div>
            {/each}
            {#if recentBuilds.length === 0 && !loading}
              <div class="frow"><span class="muted">{t('overview.noBuilds')}</span></div>
            {/if}
          </div>
        </div>

        <div class="seclabel">{t('overview.controls')}</div>
        <div class="controls">
          <button class="primary" onclick={newPack}>{t('packs.new')}</button>
        </div>
      </section>
    {:else if route.section === 'packs'}
      {#if packEdit !== null}
        {#key packEdit}
          <PackEditor
            packId={packEdit}
            onClose={() => {
              packEdit = null;
              loadAll();
            }}
          />
        {/key}
      {:else}
        <div class="bar">
          <button class="primary" onclick={newPack}>{t('packs.new')}</button>
        </div>
        <div class="panel">
          <table>
            <thead>
              <tr>
                <th>{t('packs.col.pack')}</th>
                <th>{t('packs.col.mc')}</th>
                <th>{t('packs.col.latest')}</th>
                <th>{t('packs.col.state')}</th>
                <th>{t('packs.col.tags')}</th>
                <th>{t('packs.col.flags')}</th>
                <th style="width:90px"></th>
              </tr>
            </thead>
            <tbody>
              {#each allPackIds as id}
                {@const p = summaryFor(id)}
                <tr
                  class="clickable"
                  role="button"
                  tabindex="0"
                  onclick={() => (packEdit = id)}
                  onkeydown={(e) => {
                    if (e.target !== e.currentTarget) return;
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      packEdit = id;
                    }
                  }}
                >
                  <td>
                    <div>{p?.display_name ?? id}</div>
                    {#if (p?.display_name ?? id) !== id}
                      <div class="faint mono">{id}</div>
                    {/if}
                  </td>
                  <td class="mono">{p?.minecraft_version ?? '-'}</td>
                  <td class="mono">{p?.latest_pack_version ?? t('packs.unbuilt')}</td>
                  <td>
                    {#if p}
                      <span class="tag vis-{p.visibility}">{t(visKey[p.visibility])}</span>
                      {#if p.tier === 'community'}<span class="tag">{t('packs.tier.community')}</span>{/if}
                    {/if}
                  </td>
                  <td>{#each p?.tags ?? [] as tg}<span class="tag">{tg}</span> {/each}</td>
                  <td>
                    {#if p?.featured}<span class="tag" style="color:var(--accent)">{t('packs.flag.featured')}</span>{/if}
                    {#if authoringSet.has(id)}<span class="tag" style="color:var(--ok)">{t('packs.flag.authoring')}</span>{/if}
                  </td>
                  <td class="actions">
                    {#if p}
                      <button
                        onclick={(e) => {
                          e.stopPropagation();
                          togglePublish(p);
                        }}>{p.visibility === 'published' ? t('packs.unpublish') : t('packs.publish')}</button>
                    {/if}
                    <button
                      class="danger"
                      onclick={(e) => {
                        e.stopPropagation();
                        delPack(id);
                      }}>{t('common.delete')}</button>
                  </td>
                </tr>
              {/each}
              {#if allPackIds.length === 0 && !loading}
                <tr><td colspan="7" class="muted">{t('packs.empty')}</td></tr>
              {/if}
            </tbody>
          </table>
        </div>
      {/if}
    {:else if route.section === 'servers'}
      {#if serverEdit !== null}
        {#key serverEdit}
          <ServerEditor
            initial={serverEdit === 'new' ? null : serverEdit}
            packIds={packs.map((p) => p.pack_id)}
            onSaved={() => {
              serverEdit = null;
              loadAll();
            }}
            onCancel={() => (serverEdit = null)}
          />
        {/key}
      {:else}
        <div class="bar">
          <button class="primary" onclick={() => (serverEdit = 'new')}>{t('servers.new')}</button>
        </div>
        <div class="panel">
          <table>
            <thead>
              <tr>
                <th>{t('servers.col.server')}</th>
                <th>{t('packs.col.pack')}</th>
                <th>{t('servers.col.owner')}</th>
                <th>{t('packs.col.flags')}</th>
                <th style="width:90px"></th>
              </tr>
            </thead>
            <tbody>
              {#each servers as s}
                <tr
                  class="clickable"
                  role="button"
                  tabindex="0"
                  onclick={() => (serverEdit = s)}
                  onkeydown={(e) => {
                    if (e.target !== e.currentTarget) return;
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      serverEdit = s;
                    }
                  }}
                >
                  <td>
                    <div>{s.display_name}</div>
                    <div class="faint mono">{s.server_id}</div>
                  </td>
                  <td class="mono">{s.pack_id}</td>
                  <td>{s.owner_display}</td>
                  <td>
                    {#if s.featured}<span class="tag" style="color:var(--accent)">{t('packs.flag.featured')}</span>{/if}
                  </td>
                  <td class="actions">
                    <button
                      class="danger"
                      onclick={(e) => {
                        e.stopPropagation();
                        delServer(s.server_id);
                      }}>{t('common.delete')}</button>
                  </td>
                </tr>
              {/each}
              {#if servers.length === 0 && !loading}
                <tr><td colspan="5" class="muted">{t('servers.empty')}</td></tr>
              {/if}
            </tbody>
          </table>
        </div>
      {/if}
    {:else if route.section === 'mods'}
      <ModManager />
    {:else if route.section === 'users'}
      <UsersView />
    {/if}
  </div>
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .body {
    min-width: 0;
  }
  tr.clickable {
    cursor: pointer;
  }
  tr.clickable:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .err {
    color: var(--danger);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: var(--radius-sm);
    padding: var(--space-3) var(--space-4);
    font-size: 12px;
  }
  .ov {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .readout {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(158px, 1fr));
    border: 1px solid var(--seam);
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--panel);
    box-shadow: var(--shadow-1);
  }
  .stat {
    padding: var(--space-4);
    border-right: 1px solid var(--seam);
  }
  .stat:last-child {
    border-right: none;
  }
  .stat .k {
    font-size: 12px;
    color: var(--fg-dim);
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .stat .k .d {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--red);
  }
  .stat .v {
    font-family: var(--mono);
    font-size: 30px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0;
    margin-top: 6px;
  }
  .stat .v .unit {
    font-size: 15px;
    font-weight: 600;
    color: var(--fg-dim);
    margin-left: 3px;
  }
  .stat .s {
    font-size: 11.5px;
    color: var(--fg-faint);
    margin-top: 3px;
  }
  .cols {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-4);
  }
  @media (max-width: 760px) {
    .cols {
      grid-template-columns: 1fr;
    }
  }
  .card {
    border: 1px solid var(--seam);
    border-radius: var(--radius-md);
    background: var(--panel);
    overflow: hidden;
    box-shadow: var(--shadow-1);
  }
  .card h3 {
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--fg-dim);
    margin: 0;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--seam);
  }
  .lrow {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 11px var(--space-4);
    border-bottom: 1px solid var(--seam);
  }
  .lrow:last-child {
    border-bottom: none;
  }
  .lrow.clickable {
    cursor: pointer;
  }
  .lrow.clickable:hover {
    background: var(--panel-2);
  }
  .lrow.clickable:focus-visible {
    outline: 2px solid var(--fg);
    outline-offset: -2px;
  }
  .lrow .av {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    background: var(--panel-3);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    font-weight: 700;
    color: var(--fg-dim);
    flex: none;
  }
  .lcol {
    min-width: 0;
  }
  .lrow .nm {
    font-weight: 600;
    font-size: 13.5px;
  }
  .lrow .mm {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--fg-faint);
  }
  .lrow .grow {
    flex: 1;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    font-size: 11.5px;
    font-weight: 600;
    padding: 3px 10px;
    border-radius: 999px;
    background: var(--panel-2);
    color: var(--fg-dim);
  }
  .chip .g {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--fg-faint);
  }
  .chip.ok {
    color: var(--ok);
    background: var(--ok-soft);
  }
  .chip.ok .g {
    background: var(--ok);
  }
  .seclabel {
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--fg-faint);
    margin-bottom: -4px;
  }
  .frow {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    padding: 10px var(--space-4);
    border-bottom: 1px solid var(--seam);
    font-size: 13px;
  }
  .frow:last-child {
    border-bottom: none;
  }
  .frow .ft {
    color: var(--fg-faint);
    font-size: 11px;
    white-space: nowrap;
    font-family: var(--mono);
  }
  .frow .fx {
    color: var(--fg-dim);
  }
  .frow .fx b {
    color: var(--fg);
    font-weight: 600;
  }
  .controls {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
    align-items: center;
  }
  .bar {
    margin-bottom: 14px;
  }
  .actions {
    white-space: nowrap;
  }
  .actions button {
    padding: 4px 10px;
    font-size: 12px;
    margin-right: 6px;
  }
  .vis-published {
    color: var(--ok);
  }
  .vis-draft {
    color: var(--fg-faint);
  }
  .vis-unlisted {
    color: var(--accent);
  }
  button.danger:hover {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
