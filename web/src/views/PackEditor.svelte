<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import type {
    DeclaredAsset,
    DeclaredMod,
    JobStatus,
    PackConfig,
    SourceDecl,
    ValidateReport,
  } from '../lib/types';
  import BuildConsole from './BuildConsole.svelte';
  import BrandingEditor from './BrandingEditor.svelte';
  import CuratorEditor from './CuratorEditor.svelte';
  import JobLog from './JobLog.svelte';
  import ModrinthPicker from './ModrinthPicker.svelte';
  import PackPreview from './PackPreview.svelte';

  let { packId, onClose }: { packId: string; onClose: () => void } = $props();

  type Section = 'config' | 'curator' | 'branding' | 'build';
  let section = $state<Section>('config');
  let previewOpen = $state(false);

  // bootstrap-from-SC-archive (only shown when there is no config yet)
  let bootstrapMode = $state(false);
  let bootMc = $state('1.12.2');
  let bootLoader = $state('');
  let bootName = $state('');
  let bootBusy = $state(false);
  let bootJobId = $state<string | null>(null);

  // Modrinth picker open for config-mods row index N, null = closed
  let modPicker = $state<number | null>(null);

  let cfg = $state<PackConfig | null>(null);
  let tagsStr = $state('');
  let loading = $state(true);
  let err = $state('');
  let cfgMsg = $state('');
  let savingCfg = $state(false);

  // validate the saved config against an uploaded SC archive
  let validating = $state(false);
  let valReport = $state<ValidateReport | null>(null);
  let valErr = $state('');

  async function onValidate(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    validating = true;
    valErr = '';
    valReport = null;
    try {
      valReport = await api.validatePack(packId, file);
    } catch (x) {
      valErr = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    } finally {
      validating = false;
      input.value = '';
    }
  }

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

  function newSource(type: SourceDecl['type']): SourceDecl {
    if (type === 'modrinth') return { type, project_id: '', version_id: '' };
    if (type === 'smrt_cache') return { type, sha1: '' };
    return { type, rel_path: '' };
  }
  function addMod() {
    cfg!.mods = [
      ...cfg!.mods,
      {
        filename: '',
        required: true,
        default_enabled: true,
        source: { type: 'smrt_cache', sha1: '' },
      },
    ];
  }
  function addAsset() {
    cfg!.assets = [
      ...(cfg!.assets ?? []),
      {
        dest: '',
        required: true,
        source: { type: 'smrt_static', rel_path: '' },
      },
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
  {#if !loading && cfg}
    <button class="pv" class:active={previewOpen} onclick={() => (previewOpen = !previewOpen)}>
      {previewOpen ? 'Hide preview' : 'Preview'}
    </button>
  {/if}
  <button onclick={onClose}>Close</button>
</div>

{#if err}<div class="err mono">{err}</div>{/if}

<div class="body" class:split={previewOpen}>
  <div class="editcol">
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
      <label class="valbtn">
        {validating ? 'validating...' : 'Validate vs SC archive'}
        <input type="file" accept=".zip" onchange={onValidate} disabled={validating} hidden />
      </label>
      {#if cfgMsg}<span class="muted mono">{cfgMsg}</span>{/if}
    </div>

    {#if valErr}<div class="err mono">{valErr}</div>{/if}
    {#if valReport}
      <div class="panel valrep">
        <div class="valhead">
          <span style="color:var(--ok)">{valReport.matched} matched</span>
          <span style={valReport.missing_in_config.length ? 'color:var(--danger)' : 'opacity:.62'}>
            {valReport.missing_in_config.length} missing in config
          </span>
          <span class="faint">{valReport.extra_in_config.length} extra (curator additions)</span>
          <span class="faint">SC archive: {valReport.sc_mod_count} mods</span>
        </div>
        {#if valReport.missing_in_config.length}
          <div class="vallist">
            <div class="vl-h" style="color:var(--danger)">
              Missing in config (in the archive -- would break the FML handshake)
            </div>
            {#each valReport.missing_in_config as m}<div class="mono vl-row">{m}</div>{/each}
          </div>
        {/if}
        {#if valReport.extra_in_config.length}
          <div class="vallist">
            <div class="vl-h faint">Extra in config (declared on top of the SC set)</div>
            {#each valReport.extra_in_config as m}<div class="mono vl-row">{m}</div>{/each}
          </div>
        {/if}
      </div>
    {/if}

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
          <tr><th>filename</th><th style="width:130px">source</th><th>ref</th><th style="width:60px" title="required -- always installed; the player cannot turn it off">req</th><th style="width:60px" title="default_enabled -- starting on/off state for an optional (req-off) mod; ignored when required">def</th><th>note</th><th style="width:44px"></th></tr>
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
                  <button class="sm" type="button" onclick={() => (modPicker = i)}>find on Modrinth</button>
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
          <tr><th>dest</th><th style="width:130px">source</th><th>ref</th><th style="width:60px" title="required -- always installed">req</th><th style="width:44px"></th></tr>
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
  {#if cfg}
    <CuratorEditor packId={packId} mods={cfg.mods.map((m) => m.filename)} mc={cfg.minecraft_version} />
  {:else}
    <div class="muted">Create or bootstrap a config first (Config tab).</div>
  {/if}
{:else if section === 'branding'}
  <BrandingEditor {packId} />
{:else if section === 'build'}
  <BuildConsole {packId} />
{/if}
  </div>
  {#if previewOpen}
    <div class="previewcol">
      <PackPreview {packId} />
    </div>
  {/if}
</div>

{#if modPicker !== null && cfg}
  <ModrinthPicker
    mc={cfg.minecraft_version}
    onClose={() => (modPicker = null)}
    onPick={(sel) => {
      const m = cfg!.mods[modPicker!];
      m.source = { type: 'modrinth', project_id: sel.project_id, version_id: sel.version_id };
      if (!m.filename) m.filename = `${sel.slug}.jar`;
      modPicker = null;
    }}
  />
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
  button.sm {
    padding: 3px 9px;
    font-size: 12px;
  }
  button.danger:hover {
    border-color: var(--danger);
    color: var(--danger);
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
  .pv {
    margin-right: 8px;
  }
  .pv.active {
    border-color: var(--accent);
    color: var(--accent);
  }
  .body.split {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 18px;
    align-items: start;
  }
  .editcol {
    min-width: 0;
  }
  .previewcol {
    position: sticky;
    top: 12px;
    max-height: calc(100vh - 96px);
    overflow: auto;
    min-width: 0;
  }
  .valbtn {
    display: inline-block;
    font-size: 13px;
    color: var(--fg);
    background: var(--panel-2);
    border: 1px solid var(--seam-bright);
    padding: 6px 12px;
    cursor: pointer;
  }
  .valbtn:hover {
    border-color: var(--accent);
  }
  .valrep {
    padding: 14px;
    margin-bottom: 20px;
  }
  .valhead {
    display: flex;
    gap: 16px;
    flex-wrap: wrap;
    font-size: 12px;
  }
  .vallist {
    margin-top: 12px;
  }
  .vl-h {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: 6px;
  }
  .vl-row {
    font-size: 12px;
    padding: 2px 0;
    color: var(--fg);
  }
</style>
