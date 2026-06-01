<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import type { DeclaredAsset, DeclaredMod, JobStatus, PackConfig, SourceDecl } from '../lib/types';
  import BuildConsole from './BuildConsole.svelte';
  import BrandingEditor from './BrandingEditor.svelte';
  import JobLog from './JobLog.svelte';

  let { packId, onClose }: { packId: string; onClose: () => void } = $props();

  type Section = 'config' | 'curator' | 'branding' | 'build';
  let section = $state<Section>('config');

  // bootstrap-from-SC-archive (only shown when there is no config yet)
  let bootstrapMode = $state(false);
  let bootMc = $state('1.12.2');
  let bootLoader = $state('');
  let bootName = $state('');
  let bootBusy = $state(false);
  let bootJobId = $state<string | null>(null);

  let cfg = $state<PackConfig | null>(null);
  let tagsStr = $state('');
  let curatorText = $state('');
  let loading = $state(true);
  let err = $state('');
  let cfgMsg = $state('');
  let curMsg = $state('');
  let savingCfg = $state(false);
  let savingCur = $state(false);

  async function load() {
    loading = true;
    err = '';
    try {
      const c = await api.packConfig(packId);
      cfg = c;
      tagsStr = (c.tags ?? []).join(', ');
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        cfg = null; // offer to create
      } else {
        err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
      }
    }
    try {
      curatorText = await api.curator(packId);
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 404)) {
        err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
      }
      curatorText = curatorText || '';
    }
    loading = false;
  }
  load();

  function createBlank() {
    cfg = {
      pack_id: packId,
      display_name: packId,
      tagline: '',
      minecraft_version: '1.12.2',
      loader: { name: 'forge', version: '' },
      java_major: 8,
      tags: [],
      featured: false,
      mods: [],
      assets: [],
    };
    tagsStr = '';
  }

  async function onBootstrap(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    bootBusy = true;
    err = '';
    bootJobId = null;
    try {
      const { job_id } = await api.bootstrapPack(
        packId,
        {
          minecraft_version: bootMc.trim(),
          loader_version: bootLoader.trim(),
          display_name: bootName.trim() || undefined,
        },
        file,
      );
      bootJobId = job_id;
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
      bootBusy = false;
    } finally {
      input.value = '';
    }
  }

  function onBootDone(status: JobStatus) {
    bootBusy = false;
    if (status === 'done') {
      bootstrapMode = false;
      load();
    }
  }

  async function saveConfig() {
    if (!cfg) return;
    savingCfg = true;
    cfgMsg = '';
    const payload: PackConfig = {
      ...$state.snapshot(cfg),
      tags: tagsStr
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean),
    };
    try {
      await api.savePackConfig(packId, payload);
      cfgMsg = 'Saved.';
    } catch (e) {
      cfgMsg = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      savingCfg = false;
    }
  }

  async function saveCurator() {
    savingCur = true;
    curMsg = '';
    try {
      await api.saveCurator(packId, curatorText);
      curMsg = 'Saved -- comments preserved.';
    } catch (e) {
      curMsg = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      savingCur = false;
    }
  }

  function newSource(type: SourceDecl['type']): SourceDecl {
    if (type === 'modrinth') return { type, project_id: '', version_id: '' };
    if (type === 'smrt_cache') return { type, sha1: '' };
    return { type, rel_path: '' };
  }
  function addMod() {
    cfg!.mods = [
      ...cfg!.mods,
      { filename: '', required: true, default_enabled: true, source: { type: 'smrt_cache', sha1: '' } },
    ];
  }
  function addAsset() {
    cfg!.assets = [
      ...(cfg!.assets ?? []),
      { dest: '', required: true, source: { type: 'smrt_static', rel_path: '' } },
    ];
  }
</script>

<div class="hd">
  <h2 class="ttl mono">{packId}<span class="faint">/edit</span></h2>
  <nav class="sub">
    {#each [['config', 'Config'], ['curator', 'Curator'], ['branding', 'Branding'], ['build', 'Build']] as [id, label]}
      <button class="seg" class:active={section === id} onclick={() => (section = id as Section)}>{label}</button>
    {/each}
  </nav>
  <div class="spacer"></div>
  <button onclick={onClose}>Close</button>
</div>

{#if err}<div class="err mono">{err}</div>{/if}

{#if loading}
  <div class="muted mono">loading...</div>
{:else if section === 'config'}
  {#if !cfg}
    <div class="panel empty">
      <p class="muted">No authoring config for <span class="mono">{packId}</span> yet.</p>
      <div class="opts">
        <button class="primary" onclick={createBlank}>Create blank config</button>
        <button onclick={() => (bootstrapMode = !bootstrapMode)}>Bootstrap from SC archive</button>
      </div>
      {#if bootstrapMode}
        <div class="bootform">
          <div class="brow">
            <label>minecraft_version<input bind:value={bootMc} placeholder="1.12.2" /></label>
            <label>loader_version<input bind:value={bootLoader} placeholder="14.23.5.2922" /></label>
            <label>display_name<input bind:value={bootName} placeholder={packId} /></label>
          </div>
          <label class="upbtn">
            {bootBusy ? 'working...' : 'Choose SC archive (.zip) + bootstrap'}
            <input
              type="file"
              accept=".zip"
              onchange={onBootstrap}
              disabled={bootBusy || !bootMc.trim() || !bootLoader.trim()}
              hidden
            />
          </label>
          {#if bootJobId}{#key bootJobId}<JobLog jobId={bootJobId} onDone={onBootDone} />{/key}{/if}
        </div>
      {/if}
    </div>
  {:else}
    <div class="bar row">
      <button class="primary" onclick={saveConfig} disabled={savingCfg}>
        {savingCfg ? 'saving...' : 'Save config'}
      </button>
      {#if cfgMsg}<span class="muted mono">{cfgMsg}</span>{/if}
    </div>

    <div class="panel meta">
      <label>display_name<input bind:value={cfg.display_name} /></label>
      <label>minecraft_version<input bind:value={cfg.minecraft_version} /></label>
      <label>loader.name<input bind:value={cfg.loader.name} /></label>
      <label>loader.version<input bind:value={cfg.loader.version} /></label>
      <label>java_major<input type="number" bind:value={cfg.java_major} /></label>
      <label class="chk"><input type="checkbox" bind:checked={cfg.featured} /> featured</label>
      <label class="wide">tagline<input bind:value={cfg.tagline} /></label>
      <label class="wide">tags (comma)<input bind:value={tagsStr} /></label>
    </div>

    <div class="sec-h row">
      <h3>Mods <span class="faint">({cfg.mods.length})</span></h3>
      <button onclick={addMod}>Add mod</button>
    </div>
    <div class="panel scroll">
      <table>
        <thead>
          <tr><th>filename</th><th style="width:130px">source</th><th>ref</th><th style="width:60px">req</th><th style="width:60px">def</th><th>note</th><th style="width:44px"></th></tr>
        </thead>
        <tbody>
          {#each cfg.mods as m, i}
            <tr>
              <td><input class="mono" bind:value={m.filename} /></td>
              <td>
                <select value={m.source.type} onchange={(e) => (cfg!.mods[i].source = newSource((e.currentTarget as HTMLSelectElement).value as SourceDecl['type']))}>
                  <option value="smrt_cache">smrt_cache</option>
                  <option value="modrinth">modrinth</option>
                  <option value="smrt_static">smrt_static</option>
                </select>
              </td>
              <td>
                {#if m.source.type === 'modrinth'}
                  <input class="mono" bind:value={m.source.project_id} placeholder="project_id" />
                  <input class="mono" bind:value={m.source.version_id} placeholder="version_id" />
                {:else if m.source.type === 'smrt_cache'}
                  <input class="mono" bind:value={m.source.sha1} placeholder="sha1" />
                {:else}
                  <input class="mono" bind:value={m.source.rel_path} placeholder="rel_path" />
                {/if}
              </td>
              <td class="ctr"><input type="checkbox" bind:checked={m.required} /></td>
              <td class="ctr"><input type="checkbox" bind:checked={m.default_enabled} /></td>
              <td><input bind:value={m.note} placeholder="rationale / note" /></td>
              <td class="ctr"><button class="danger sm" onclick={() => (cfg!.mods = cfg!.mods.filter((_, j) => j !== i))}>x</button></td>
            </tr>
          {/each}
          {#if cfg.mods.length === 0}
            <tr><td colspan="7" class="muted">No mods declared. Add one or bootstrap from an SC archive.</td></tr>
          {/if}
        </tbody>
      </table>
    </div>

    <div class="sec-h row">
      <h3>Assets <span class="faint">({(cfg.assets ?? []).length})</span></h3>
      <button onclick={addAsset}>Add asset</button>
    </div>
    <div class="panel scroll">
      <table>
        <thead>
          <tr><th>dest</th><th style="width:130px">source</th><th>ref</th><th style="width:60px">req</th><th style="width:44px"></th></tr>
        </thead>
        <tbody>
          {#each cfg.assets ?? [] as a, i}
            <tr>
              <td><input class="mono" bind:value={a.dest} /></td>
              <td>
                <select value={a.source.type} onchange={(e) => (cfg!.assets![i].source = newSource((e.currentTarget as HTMLSelectElement).value as SourceDecl['type']))}>
                  <option value="smrt_static">smrt_static</option>
                  <option value="modrinth">modrinth</option>
                  <option value="smrt_cache">smrt_cache</option>
                </select>
              </td>
              <td>
                {#if a.source.type === 'modrinth'}
                  <input class="mono" bind:value={a.source.project_id} placeholder="project_id" />
                  <input class="mono" bind:value={a.source.version_id} placeholder="version_id" />
                {:else if a.source.type === 'smrt_cache'}
                  <input class="mono" bind:value={a.source.sha1} placeholder="sha1" />
                {:else}
                  <input class="mono" bind:value={a.source.rel_path} placeholder="rel_path" />
                {/if}
              </td>
              <td class="ctr"><input type="checkbox" bind:checked={a.required} /></td>
              <td class="ctr"><button class="danger sm" onclick={() => (cfg!.assets = (cfg!.assets ?? []).filter((_, j) => j !== i))}>x</button></td>
            </tr>
          {/each}
          {#if (cfg.assets ?? []).length === 0}
            <tr><td colspan="5" class="muted">No assets declared.</td></tr>
          {/if}
        </tbody>
      </table>
    </div>
  {/if}
{:else if section === 'curator'}
  <div class="bar row">
    <button class="primary" onclick={saveCurator} disabled={savingCur}>
      {savingCur ? 'saving...' : 'Save curator.toml'}
    </button>
    {#if curMsg}<span class="muted mono">{curMsg}</span>{/if}
  </div>
  <p class="muted hint">
    The omnibus curator file: default_off, mark_optional, incompatible,
    substitute, role_table, category_table, extra_mods/assets, drop_assets,
    hidemymods. Saved verbatim -- your comments and rationale stay intact.
  </p>
  <textarea class="curator mono" bind:value={curatorText} spellcheck="false" placeholder="# curator.toml"></textarea>
{:else if section === 'branding'}
  <BrandingEditor {packId} />
{:else if section === 'build'}
  <BuildConsole {packId} />
{/if}

<style>
  .hd {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 16px;
  }
  .ttl {
    font-size: 16px;
  }
  .sub {
    display: flex;
    gap: 2px;
  }
  .seg {
    background: transparent;
    border: 1px solid transparent;
    border-bottom: 2px solid transparent;
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
  .err {
    color: var(--danger);
    font-size: 12px;
    margin-bottom: 12px;
  }
  .bar {
    margin-bottom: 14px;
  }
  .empty {
    padding: 24px;
    text-align: center;
  }
  .empty button {
    margin-top: 12px;
  }
  .meta {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px 16px;
    padding: 16px;
    margin-bottom: 20px;
  }
  .meta label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--fg-dim);
  }
  .meta label.wide {
    grid-column: 1 / -1;
  }
  .meta label.chk {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    color: var(--fg);
    align-self: end;
  }
  .meta label.chk input {
    width: auto;
  }
  .sec-h {
    margin: 0 0 10px;
    gap: 12px;
  }
  .sec-h h3 {
    font-size: 13px;
    color: var(--fg-dim);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  td input {
    padding: 5px 7px;
    font-size: 12px;
    margin-bottom: 3px;
  }
  td select {
    padding: 5px;
    font-size: 12px;
    background: var(--bg);
    color: var(--fg);
    border: 1px solid var(--seam-bright);
    border-radius: 0;
    width: 100%;
  }
  td.ctr {
    text-align: center;
  }
  td.ctr input {
    width: auto;
  }
  button.sm {
    padding: 3px 9px;
    font-size: 12px;
  }
  button.danger:hover {
    border-color: var(--danger);
    color: var(--danger);
  }
  .hint {
    font-size: 12px;
    margin: 0 0 12px;
    max-width: 720px;
  }
  .curator {
    width: 100%;
    min-height: 460px;
    font-size: 12.5px;
    line-height: 1.55;
    resize: vertical;
  }
  .panel.scroll {
    margin-bottom: 22px;
  }
  .opts {
    display: flex;
    justify-content: center;
    gap: 10px;
    margin-top: 12px;
  }
  .bootform {
    margin-top: 18px;
    text-align: left;
  }
  .brow {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
    margin-bottom: 12px;
  }
  .brow label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--fg-dim);
  }
  .upbtn {
    display: inline-block;
    font-size: 13px;
    color: var(--fg);
    background: var(--panel-2);
    border: 1px solid var(--seam-bright);
    padding: 8px 14px;
    cursor: pointer;
  }
  .upbtn:hover {
    border-color: var(--accent);
  }
</style>
