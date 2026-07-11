<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { dialogs } from '../lib/dialogs.svelte';
  import { route } from '../lib/route.svelte';
  import { t } from '../lib/i18n.svelte';
  import type { ModSummary, PackSummary, ServerEntry, UnassignedJar } from '../lib/types';
  import ServerEditor from './ServerEditor.svelte';
  import PackEditor from './PackEditor.svelte';
  import ModManager from './ModManager.svelte';

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
  let err = $state('');
  let loading = $state(true);

  async function loadAll() {
    loading = true;
    err = '';
    try {
      const [p, s, md, u, a, rm] = await Promise.all([
        api.packs(),
        api.servers(),
        api.registryMods(),
        api.unassigned(),
        api.authoringPacks(),
        api.removed(),
      ]);
      packs = p.packs;
      servers = s.servers;
      mods = md;
      unassigned = u;
      authoring = a.packs;
      removed = rm.removed;
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      loading = false;
    }
  }
  loadAll();

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

  async function newPack() {
    const id = (
      await dialogs.prompt(t('packs.newPrompt'), { title: t('packs.new') })
    )?.trim();
    if (id) packEdit = id;
  }
</script>

<div class="view">
  <div class="toolbar">
    <button onclick={loadAll} disabled={loading}>
      {loading ? t('common.loading') : t('shell.refresh')}
    </button>
  </div>

  {#if err}<div class="err mono">{err}</div>{/if}

  <div class="body">
    {#if route.section === 'overview'}
      <section class="ov">
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
            <div class="k">{t('overview.servers')}</div>
            <div class="v">{servers.length}</div>
            <div class="s">{t('overview.featured')}: {featPackCount} / {featServerCount}</div>
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
            <h3>{t('overview.servers')}</h3>
            {#each servers as s}
              <div
                class="lrow clickable"
                role="button"
                tabindex="0"
                onclick={() => {
                  route.go('servers');
                  serverEdit = s;
                }}
                onkeydown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    route.go('servers');
                    serverEdit = s;
                  }
                }}
              >
                <div class="lcol">
                  <div class="nm">{s.display_name}</div>
                  <div class="mm">{s.pack_id} &middot; {s.owner_display}</div>
                </div>
                <div class="grow"></div>
                {#if s.featured}<span class="chip ok"><span class="g"></span>{t('packs.flag.featured')}</span>{/if}
              </div>
            {/each}
            {#if servers.length === 0 && !loading}
              <div class="lrow"><span class="muted">{t('servers.empty')}</span></div>
            {/if}
          </div>
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
                <th>{t('packs.col.tags')}</th>
                <th>{t('packs.col.flags')}</th>
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
                  <td>{#each p?.tags ?? [] as tg}<span class="tag">{tg}</span> {/each}</td>
                  <td>
                    {#if p?.featured}<span class="tag" style="color:var(--accent)">{t('packs.flag.featured')}</span>{/if}
                    {#if authoringSet.has(id)}<span class="tag" style="color:var(--ok)">{t('packs.flag.authoring')}</span>{/if}
                  </td>
                </tr>
              {/each}
              {#if allPackIds.length === 0 && !loading}
                <tr><td colspan="5" class="muted">{t('packs.empty')}</td></tr>
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
    {/if}
  </div>
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .toolbar {
    display: flex;
    justify-content: flex-end;
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
    font-size: 30px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
    margin-top: 6px;
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
    font-size: 12px;
    font-weight: 600;
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
    color: var(--fg);
  }
  .chip.ok .g {
    background: var(--fg);
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
  button.danger:hover {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
