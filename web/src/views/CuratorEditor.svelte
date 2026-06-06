<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import type { Curator } from '../lib/types';
  import ModrinthPicker from './ModrinthPicker.svelte';

  let { packId, mods, mc }: { packId: string; mods: string[]; mc: string } = $props();

  let view = $state<'structured' | 'raw'>('structured');
  let curator = $state<Curator | null>(null);
  let rawText = $state('');
  let galleryStr = $state('');
  let loading = $state(true);
  let busy = $state(false);
  let err = $state('');
  let msg = $state('');
  let extraPicker = $state(false);

  async function load() {
    loading = true;
    err = '';
    try {
      const c = await api.curatorStructured(packId);
      curator = c;
      galleryStr = c.pack_meta.gallery_urls.join('\n');
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
    try {
      rawText = await api.curator(packId);
    } catch {
      rawText = '';
    }
    loading = false;
  }
  load();

  const has = (arr: string[], v: string) => arr.includes(v);
  function toggle(arr: string[], v: string) {
    const i = arr.indexOf(v);
    if (i >= 0) arr.splice(i, 1);
    else arr.push(v);
  }
  function setRec(rec: Partial<Record<string, string>>, k: string, v: string) {
    if (v.trim()) rec[k] = v.trim();
    else delete rec[k];
  }

  async function saveStructured() {
    if (!curator) return;
    busy = true;
    msg = '';
    curator.pack_meta.gallery_urls = galleryStr
      .split('\n')
      .map((s) => s.trim())
      .filter(Boolean);
    try {
      await api.saveCuratorStructured(packId, $state.snapshot(curator));
      msg = 'Saved -- section comments kept.';
      rawText = await api.curator(packId);
    } catch (e) {
      msg = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      busy = false;
    }
  }

  async function saveRaw() {
    busy = true;
    msg = '';
    try {
      await api.saveCurator(packId, rawText);
      msg = 'Saved verbatim.';
      const c = await api.curatorStructured(packId);
      curator = c;
      galleryStr = c.pack_meta.gallery_urls.join('\n');
    } catch (e) {
      msg = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="bar row">
  <div class="seg-group">
    <button class="seg" class:active={view === 'structured'} onclick={() => (view = 'structured')}>Structured</button>
    <button class="seg" class:active={view === 'raw'} onclick={() => (view = 'raw')}>Raw</button>
  </div>
  {#if view === 'structured'}
    <button class="primary" onclick={saveStructured} disabled={busy || !curator}>
      {busy ? 'saving...' : 'Save curator'}
    </button>
  {:else}
    <button class="primary" onclick={saveRaw} disabled={busy}>
      {busy ? 'saving...' : 'Save curator.toml'}
    </button>
  {/if}
  {#if msg}<span class="muted mono">{msg}</span>{/if}
</div>

{#if err}<div class="err mono">{err}</div>{/if}

{#if loading}
  <div class="muted mono">loading...</div>
{:else if view === 'raw'}
  <p class="muted hint">
    Full-fidelity TOML -- every comment kept verbatim. The omnibus file:
    default_off, mark_optional, incompatible, substitute, role_table,
    category_table, extra_mods/assets, drop_assets, hidemymods.
  </p>
  <textarea class="curator mono" bind:value={rawText} spellcheck="false" placeholder="# curator.toml"></textarea>
{:else if curator}
  <p class="muted hint">
    Structured edits re-serialize the managed tables (section comments kept,
    inner per-line comments may not). substitute / incompatible / drop_assets /
    hidemymods live in the Raw view.
  </p>

  <div class="sec-h"><h3>Pack meta</h3></div>
  <div class="panel meta">
    <label>icon_url<input class="mono" bind:value={curator.pack_meta.icon_url} placeholder="https://.../icon.png" /></label>
    <label>banner_url<input class="mono" bind:value={curator.pack_meta.banner_url} placeholder="https://.../banner.png" /></label>
    <label class="wide">gallery_urls (one per line)<textarea class="mono" rows="3" bind:value={galleryStr}></textarea></label>
    <label class="wide">description_md<textarea class="mono" rows="5" bind:value={curator.pack_meta.description_md}></textarea></label>
  </div>

  <div class="sec-h"><h3>Per-mod rules <span class="faint">({mods.length} mods)</span></h3></div>
  <div class="panel scroll">
    <table>
      <thead>
        <tr><th>mod</th><th style="width:70px">optional</th><th style="width:80px">default-off</th><th>category</th><th>role</th></tr>
      </thead>
      <tbody>
        {#each mods as m}
          <tr>
            <td class="mono">{m}</td>
            <td class="ctr"><input type="checkbox" checked={has(curator.mark_optional.filenames, m)} onchange={() => toggle(curator!.mark_optional.filenames, m)} /></td>
            <td class="ctr"><input type="checkbox" checked={has(curator.default_off, m)} onchange={() => toggle(curator!.default_off, m)} /></td>
            <td><input value={curator.category_table[m] ?? ''} oninput={(e) => setRec(curator!.category_table, m, e.currentTarget.value)} placeholder="-" /></td>
            <td><input value={curator.role_table[m] ?? ''} oninput={(e) => setRec(curator!.role_table, m, e.currentTarget.value)} placeholder="-" /></td>
          </tr>
        {/each}
        {#if mods.length === 0}<tr><td colspan="5" class="muted">No mods in the config yet.</td></tr>{/if}
      </tbody>
    </table>
  </div>

  <div class="sec-h row">
    <h3>Extra mods <span class="faint">({curator.extra_mods.length})</span></h3>
    <button onclick={() => (extraPicker = true)}>Add from Modrinth</button>
  </div>
  <div class="panel scroll">
    <table>
      <thead><tr><th>slug</th><th style="width:60px">req</th><th>category</th><th>description</th><th style="width:44px"></th></tr></thead>
      <tbody>
        {#each curator.extra_mods as em, i}
          <tr>
            <td class="mono">{em.slug}</td>
            <td class="ctr"><input type="checkbox" bind:checked={em.required} /></td>
            <td><input bind:value={em.category} placeholder="-" /></td>
            <td><input bind:value={em.description} placeholder="-" /></td>
            <td class="ctr"><button class="danger sm" onclick={() => curator!.extra_mods.splice(i, 1)}>x</button></td>
          </tr>
        {/each}
        {#if curator.extra_mods.length === 0}<tr><td colspan="5" class="muted">No extra mods. Add one from Modrinth.</td></tr>{/if}
      </tbody>
    </table>
  </div>
{/if}

{#if extraPicker && curator}
  <ModrinthPicker
    {mc}
    onClose={() => (extraPicker = false)}
    onPick={(sel) => {
      curator!.extra_mods.push({
        slug: sel.slug,
        required: true,
        category: null,
        description: null,
        name_override: null,
      });
      extraPicker = false;
    }}
  />
{/if}

<style>
  .bar {
    margin-bottom: 14px;
    gap: 12px;
  }
  .seg-group {
    display: flex;
    gap: 2px;
    border: 1px solid var(--seam-bright);
  }
  .seg {
    background: transparent;
    border: none;
    padding: 6px 14px;
    color: var(--fg-dim);
  }
  .seg.active {
    background: var(--panel-2);
    color: var(--accent);
  }
  .err {
    color: var(--danger);
    font-size: 12px;
    margin-bottom: 12px;
  }
  .hint {
    font-size: 12px;
    margin: 0 0 14px;
    max-width: 720px;
  }
  .curator {
    width: 100%;
    min-height: 460px;
    font-size: 12.5px;
    line-height: 1.55;
    resize: vertical;
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
  .meta {
    display: grid;
    grid-template-columns: 1fr 1fr;
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
  .meta textarea {
    resize: vertical;
  }
  .panel.scroll {
    margin-bottom: 22px;
  }
  td input {
    padding: 5px 7px;
    font-size: 12px;
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
</style>
