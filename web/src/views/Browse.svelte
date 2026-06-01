<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { dialogs } from '../lib/dialogs.svelte';
  import type {
    CacheInventoryEntry,
    Featured,
    Health,
    PackSummary,
    ServerEntry,
  } from '../lib/types';
  import ServerEditor from './ServerEditor.svelte';
  import PackEditor from './PackEditor.svelte';

  let { onLogout }: { onLogout: () => void } = $props();

  type Tab = 'overview' | 'packs' | 'servers' | 'featured' | 'cache';
  let tab = $state<Tab>('overview');

  // server create/edit: 'new' = creating, ServerEntry = editing, null = closed
  let serverEdit = $state<ServerEntry | 'new' | null>(null);
  // pack editor: pack_id being edited, null = closed
  let packEdit = $state<string | null>(null);

  // featured selections, synced from featured.json on load
  let featPacks = $state<Set<string>>(new Set());
  let featServers = $state<Set<string>>(new Set());
  let featBusy = $state(false);
  let featMsg = $state('');

  let health = $state<Health | null>(null);
  let packs = $state<PackSummary[]>([]);
  let servers = $state<ServerEntry[]>([]);
  let featured = $state<Featured | null>(null);
  let cache = $state<CacheInventoryEntry[]>([]);
  let removed = $state<string[]>([]);
  let authoring = $state<string[]>([]);
  let err = $state('');
  let loading = $state(true);

  // featured.json is absent on a fresh mirror; a 404 there means "nothing
  // featured yet", not an error worth banner-ing over the whole overview.
  function featuredFallback(e: unknown): Featured {
    if (e instanceof ApiError && e.status === 404) {
      return { schema_version: 2, generated_at: '', featured_servers: [], featured_packs: [] };
    }
    throw e;
  }

  async function loadAll() {
    loading = true;
    err = '';
    try {
      const [h, p, s, f, c, a, rm] = await Promise.all([
        api.health(),
        api.packs(),
        api.servers(),
        api.featured().catch(featuredFallback),
        api.cacheInventory(),
        api.authoringPacks(),
        api.removed(),
      ]);
      health = h;
      packs = p.packs;
      servers = s.servers;
      featured = f;
      featPacks = new Set(f.featured_packs);
      featServers = new Set(f.featured_servers);
      cache = c.entries;
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
    const ok = await dialogs.confirm(`Delete server "${id}"? Removes its metadata from the mirror.`, {
      title: 'Delete server',
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

  async function saveFeatured() {
    featBusy = true;
    featMsg = '';
    try {
      await api.saveFeatured({
        schema_version: 2,
        generated_at: new Date().toISOString(),
        featured_packs: [...featPacks],
        featured_servers: [...featServers],
      });
      featMsg = 'Saved.';
      await loadAll();
    } catch (e) {
      featMsg = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      featBusy = false;
    }
  }

  function toggle(set: Set<string>, id: string): Set<string> {
    const n = new Set(set);
    n.has(id) ? n.delete(id) : n.add(id);
    return n;
  }

  let uploading = $state(false);
  let upMsg = $state('');

  async function onUploadJar(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    uploading = true;
    upMsg = '';
    try {
      const sha1 = await api.uploadCacheJar(file);
      upMsg = `Uploaded ${file.name} (${sha1.slice(0, 12)}...)`;
      await loadAll();
    } catch (x) {
      upMsg = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    } finally {
      uploading = false;
      input.value = '';
    }
  }

  async function delCacheJar(sha1: string) {
    const ok = await dialogs.confirm(
      `Delete jar ${sha1.slice(0, 12)}...? It is added to the removed-list (takedown) and cannot be re-uploaded.`,
      { title: 'Delete cache jar', danger: true },
    );
    if (!ok) return;
    try {
      await api.deleteCacheJar(sha1);
      await loadAll();
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    }
  }

  const cacheBytes = $derived(cache.reduce((n, e) => n + e.size_bytes, 0));
  const authoringSet = $derived(new Set(authoring));
  const allPackIds = $derived(
    [...new Set([...packs.map((p) => p.pack_id), ...authoring])].sort(),
  );
  const summaryFor = (id: string) => packs.find((p) => p.pack_id === id);

  async function newPack() {
    const id = (
      await dialogs.prompt('New pack id (letters, digits, - _ .):', { title: 'New pack' })
    )?.trim();
    if (id) packEdit = id;
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

  const endpoints: [string, string][] = [
    ['GET', '/v1/health'],
    ['GET', '/v1/packs'],
    ['GET', '/v1/packs/:id'],
    ['GET', '/v1/packs/:id/manifest'],
    ['GET', '/v1/packs/:id/manifest/versions'],
    ['GET', '/v1/packs/:id/manifest/:version'],
    ['GET', '/v1/packs/:id/static/*path'],
    ['GET', '/v1/servers'],
    ['GET', '/v1/servers/:id'],
    ['GET', '/v1/featured'],
    ['GET', '/v1/cache/inventory'],
    ['GET', '/v1/cache/:prefix/:sha.jar'],
    ['GET', '/v1/admin/packs'],
    ['GET PUT', '/v1/admin/packs/:id/config'],
    ['GET PUT', '/v1/admin/packs/:id/curator'],
    ['GET PUT', '/v1/admin/packs/:id/curator/structured'],
    ['POST', '/v1/admin/packs/:id/build?dry_run'],
    ['POST', '/v1/admin/packs/:id/bootstrap'],
    ['POST', '/v1/admin/packs/:id/validate'],
    ['GET', '/v1/admin/jobs/:id'],
    ['GET', '/v1/admin/jobs/:id/events'],
    ['POST', '/v1/admin/servers'],
    ['DELETE', '/v1/admin/servers/:id'],
    ['PUT DELETE', '/v1/admin/cache/:prefix/:file'],
    ['GET', '/v1/admin/cache/removed'],
    ['GET PUT DELETE', '/v1/admin/packs/:id/static/*path'],
    ['POST', '/v1/admin/featured'],
    ['GET', '/v1/admin/modrinth/search | versions | icon'],
  ];

  const tabs: [Tab, string][] = [
    ['overview', 'Overview'],
    ['packs', 'Packs'],
    ['servers', 'Servers'],
    ['featured', 'Featured'],
    ['cache', 'Cache'],
  ];
</script>

<div class="shell">
  <header class="top">
    <div class="brand mono">smrt<span class="faint">/control</span></div>
    <nav class="tabs">
      {#each tabs as [id, label]}
        <button class="tab" class:active={tab === id} onclick={() => (tab = id)}>{label}</button>
      {/each}
    </nav>
    <div class="spacer"></div>
    {#if health}<span class="ver faint mono">v{health.version} / schema {health.schema_version}</span>{/if}
    <button onclick={loadAll} disabled={loading}>{loading ? '...' : 'Refresh'}</button>
    <button onclick={onLogout}>Sign out</button>
  </header>

  {#if err}<div class="err mono">{err}</div>{/if}

  <main class="body scroll">
    {#if tab === 'overview'}
      <section class="tiles">
        <div class="tile panel">
          <div class="n mono">{packs.length}</div>
          <div class="l muted">packs</div>
        </div>
        <div class="tile panel">
          <div class="n mono">{servers.length}</div>
          <div class="l muted">servers</div>
        </div>
        <div class="tile panel">
          <div class="n mono">{cache.length}</div>
          <div class="l muted">cache jars / {fmtBytes(cacheBytes)}</div>
        </div>
        <div class="tile panel">
          <div class="n mono">{authoring.length}</div>
          <div class="l muted">with authoring config</div>
        </div>
        <div class="tile panel">
          <div class="n mono">{featured?.featured_packs.length ?? 0} / {featured?.featured_servers.length ?? 0}</div>
          <div class="l muted">featured packs / servers</div>
        </div>
      </section>

      <h2 class="sec">API surface</h2>
      <div class="panel">
        <table>
          <thead><tr><th style="width:150px">Method</th><th>Path</th></tr></thead>
          <tbody>
            {#each endpoints as [method, path]}
              <tr>
                <td><span class="tag">{method}</span></td>
                <td class="mono">{path}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else if tab === 'packs'}
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
          <button class="primary" onclick={newPack}>New pack</button>
        </div>
        <div class="panel">
          <table>
            <thead>
              <tr><th>Pack</th><th>MC</th><th>Latest</th><th>Tags</th><th>Flags</th><th style="width:80px"></th></tr>
            </thead>
            <tbody>
              {#each allPackIds as id}
                {@const p = summaryFor(id)}
                <tr>
                  <td>
                    <div>{p?.display_name ?? id}</div>
                    <div class="faint mono">{id}</div>
                  </td>
                  <td class="mono">{p?.minecraft_version ?? '-'}</td>
                  <td class="mono">{p?.latest_pack_version ?? '(unbuilt)'}</td>
                  <td>{#each p?.tags ?? [] as t}<span class="tag">{t}</span> {/each}</td>
                  <td>
                    {#if p?.featured}<span class="tag" style="color:var(--accent)">featured</span>{/if}
                    {#if authoringSet.has(id)}<span class="tag" style="color:var(--ok)">authoring</span>{/if}
                  </td>
                  <td class="actions"><button onclick={() => (packEdit = id)}>Edit</button></td>
                </tr>
              {/each}
              {#if allPackIds.length === 0 && !loading}
                <tr><td colspan="6" class="muted">No packs yet. Create one or bootstrap from an SC archive.</td></tr>
              {/if}
            </tbody>
          </table>
        </div>
      {/if}
    {:else if tab === 'servers'}
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
          <button class="primary" onclick={() => (serverEdit = 'new')}>New server</button>
        </div>
      {/if}
      <div class="panel">
        <table>
          <thead>
            <tr><th>Server</th><th>Pack</th><th>Owner</th><th>Flags</th><th style="width:160px"></th></tr>
          </thead>
          <tbody>
            {#each servers as s}
              <tr>
                <td>
                  <div>{s.display_name}</div>
                  <div class="faint mono">{s.server_id}</div>
                </td>
                <td class="mono">{s.pack_id}</td>
                <td>{s.owner_display}</td>
                <td>{#if s.featured}<span class="tag" style="color:var(--accent)">featured</span>{/if}</td>
                <td class="actions">
                  <button onclick={() => (serverEdit = s)}>Edit</button>
                  <button class="danger" onclick={() => delServer(s.server_id)}>Delete</button>
                </td>
              </tr>
            {/each}
            {#if servers.length === 0 && !loading}
              <tr><td colspan="5" class="muted">No servers curated yet.</td></tr>
            {/if}
          </tbody>
        </table>
      </div>
    {:else if tab === 'featured'}
      <div class="bar row">
        <button class="primary" onclick={saveFeatured} disabled={featBusy}>
          {featBusy ? 'saving...' : 'Save featured'}
        </button>
        {#if featMsg}<span class="muted mono">{featMsg}</span>{/if}
      </div>
      <div class="feat">
        <div class="panel col">
          <div class="ch">Featured packs</div>
          {#each packs as p}
            <label class="opt">
              <input
                type="checkbox"
                checked={featPacks.has(p.pack_id)}
                onchange={() => (featPacks = toggle(featPacks, p.pack_id))}
              />
              {p.display_name} <span class="faint mono">{p.pack_id}</span>
            </label>
          {/each}
          {#if packs.length === 0}<div class="muted">No packs.</div>{/if}
        </div>
        <div class="panel col">
          <div class="ch">Featured servers</div>
          {#each servers as s}
            <label class="opt">
              <input
                type="checkbox"
                checked={featServers.has(s.server_id)}
                onchange={() => (featServers = toggle(featServers, s.server_id))}
              />
              {s.display_name} <span class="faint mono">{s.server_id}</span>
            </label>
          {/each}
          {#if servers.length === 0}<div class="muted">No servers.</div>{/if}
        </div>
      </div>
    {:else if tab === 'cache'}
      <div class="bar row">
        <label class="upbtn">
          {uploading ? 'uploading...' : 'Upload jar'}
          <input type="file" accept=".jar" onchange={onUploadJar} disabled={uploading} hidden />
        </label>
        {#if upMsg}<span class="muted mono">{upMsg}</span>{/if}
      </div>
      <div class="cache-head muted">
        {cache.length} jars, {fmtBytes(cacheBytes)} total
      </div>
      <div class="panel">
        <table>
          <thead>
            <tr><th>sha1</th><th style="width:140px">size</th><th style="width:90px"></th></tr>
          </thead>
          <tbody>
            {#each cache as c}
              <tr>
                <td class="mono">{c.sha1}</td>
                <td class="mono">{fmtBytes(c.size_bytes)}</td>
                <td class="actions">
                  <button class="danger" onclick={() => delCacheJar(c.sha1)}>Delete</button>
                </td>
              </tr>
            {/each}
            {#if cache.length === 0 && !loading}
              <tr><td colspan="3" class="muted">Cache is empty. Upload a jar to seed it.</td></tr>
            {/if}
          </tbody>
        </table>
      </div>

      {#if removed.length}
        <h2 class="sec rm">Removed (takedown)</h2>
        <div class="cache-head muted">
          {removed.length} sha1{removed.length === 1 ? '' : 's'} blocked from re-ingestion (removed.txt)
        </div>
        <div class="panel">
          <table>
            <thead><tr><th>sha1</th></tr></thead>
            <tbody>
              {#each removed as sha}
                <tr><td class="mono">{sha}</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/if}
  </main>
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--seam);
    background: var(--panel);
  }
  .brand {
    font-size: 16px;
    letter-spacing: 0.04em;
  }
  .tabs {
    display: flex;
    gap: 2px;
  }
  .tab {
    background: transparent;
    border: 1px solid transparent;
    border-bottom: 2px solid transparent;
    padding: 6px 12px;
    color: var(--fg-dim);
  }
  .tab:hover {
    color: var(--fg);
    border-color: transparent;
  }
  .tab.active {
    color: var(--fg);
    border-bottom-color: var(--accent);
  }
  .spacer {
    flex: 1;
  }
  .ver {
    font-size: 11px;
  }
  .body {
    flex: 1;
    padding: 18px 16px;
  }
  .err {
    color: var(--danger);
    padding: 8px 16px;
    border-bottom: 1px solid var(--seam);
    font-size: 12px;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 10px;
    margin-bottom: 24px;
  }
  .tile {
    padding: 16px;
  }
  .tile .n {
    font-size: 26px;
    color: var(--accent);
  }
  .tile .l {
    font-size: 12px;
    margin-top: 4px;
  }
  .sec {
    font-size: 13px;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin-bottom: 10px;
  }
  .sec.rm {
    margin-top: 26px;
  }
  .cache-head {
    font-size: 12px;
    margin-bottom: 10px;
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
  .feat {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
  }
  .col {
    padding: 14px;
  }
  .ch {
    font-size: 12px;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-bottom: 10px;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 0;
    font-size: 13px;
  }
  .opt input {
    width: auto;
  }
  .upbtn {
    display: inline-block;
    font-size: 13px;
    color: var(--fg);
    background: var(--panel-2);
    border: 1px solid var(--seam-bright);
    padding: 7px 14px;
    cursor: pointer;
  }
  .upbtn:hover {
    border-color: var(--accent);
  }
</style>
