<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { dialogs } from '../lib/dialogs.svelte';

  let { packId }: { packId: string } = $props();

  let files = $state<string[]>([]);
  let relPath = $state('_nexira/icon.png');
  let busy = $state(false);
  let err = $state('');

  async function load() {
    err = '';
    try {
      files = (await api.packStatic(packId)).files;
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 404)) {
        err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
      }
      files = [];
    }
  }
  load();

  async function onUpload(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file || !relPath.trim()) return;
    busy = true;
    err = '';
    try {
      await api.uploadStatic(packId, relPath.trim(), file);
      await load();
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    } finally {
      busy = false;
      input.value = '';
    }
  }

  async function del(f: string) {
    const ok = await dialogs.confirm(`Delete static asset "${f}"?`, {
      title: 'Delete asset',
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteStatic(packId, f);
      await load();
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    }
  }

  const isImage = (f: string) => /\.(png|jpe?g|gif|webp|svg)$/i.test(f);
</script>

<p class="muted hint">
  Pack branding + static assets (icon / banner / gallery / configs). Upload here,
  then reference the public URL in curator <span class="mono">pack_meta</span>
  (icon_url / banner_url / gallery_urls).
</p>

<div class="up panel">
  <label class="path">destination path<input class="mono" bind:value={relPath} placeholder="_nexira/icon.png" /></label>
  <label class="upbtn">
    {busy ? 'uploading...' : 'Choose file + upload'}
    <input type="file" onchange={onUpload} disabled={busy || !relPath.trim()} hidden />
  </label>
</div>

{#if err}<div class="err mono">{err}</div>{/if}

<div class="grid">
  {#each files as f}
    <div class="card panel">
      {#if isImage(f)}
        <img src={api.staticUrl(packId, f)} alt={f} />
      {:else}
        <div class="ext mono">.{f.split('.').pop()}</div>
      {/if}
      <div class="meta">
        <div class="fn mono" title={f}>{f}</div>
        <div class="row2">
          <a class="mono" href={api.staticUrl(packId, f)} target="_blank" rel="noreferrer">open</a>
          <button class="danger sm" onclick={() => del(f)}>delete</button>
        </div>
      </div>
    </div>
  {/each}
  {#if files.length === 0}<div class="muted">No static assets uploaded yet.</div>{/if}
</div>

<style>
  .hint {
    font-size: 12px;
    margin: 0 0 14px;
    max-width: 720px;
  }
  .up {
    display: flex;
    align-items: flex-end;
    gap: 14px;
    padding: 14px;
    margin-bottom: 16px;
  }
  .path {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--fg-dim);
    flex: 1;
  }
  .upbtn {
    display: inline-block;
    font-size: 13px;
    color: var(--fg);
    background: var(--panel-2);
    border: 1px solid var(--seam-bright);
    padding: 8px 14px;
    cursor: pointer;
    white-space: nowrap;
  }
  .upbtn:hover {
    border-color: var(--accent);
  }
  .err {
    color: var(--danger);
    font-size: 12px;
    margin-bottom: 12px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 12px;
  }
  .card {
    overflow: hidden;
  }
  .card img {
    width: 100%;
    height: 110px;
    object-fit: contain;
    background: var(--bg);
    display: block;
  }
  .ext {
    height: 110px;
    display: grid;
    place-items: center;
    color: var(--fg-faint);
    background: var(--bg);
    font-size: 18px;
  }
  .meta {
    padding: 8px 10px;
  }
  .fn {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row2 {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 6px;
  }
  button.danger.sm {
    padding: 3px 9px;
    font-size: 11px;
  }
  button.danger:hover {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
