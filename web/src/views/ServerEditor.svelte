<script lang="ts">
  import { untrack } from 'svelte';
  import { api, ApiError } from '../lib/api';
  import type { ServerEntry } from '../lib/types';

  let {
    initial,
    packIds,
    onSaved,
    onCancel,
  }: {
    initial: ServerEntry | null;
    packIds: string[];
    onSaved: () => void;
    onCancel: () => void;
  } = $props();

  const isNew = $derived(initial === null);

  // One-shot working copy; the parent remounts this editor per row via {#key},
  // so capturing the initial value here is intentional (hence untrack).
  let f = $state<ServerEntry>(
    untrack(() =>
      initial
        ? {
            ...initial,
            tags: [...(initial.tags ?? [])],
            gallery_urls: [...(initial.gallery_urls ?? [])],
          }
        : {
            schema_version: 2,
            server_id: '',
            pack_id: packIds[0] ?? '',
            display_name: '',
            tagline: '',
            description_md: '',
            banner_url: '',
            gallery_urls: [],
            tags: [],
            owner_display: '',
            featured: false,
          },
    ),
  );
  let tagsStr = $state(untrack(() => (initial?.tags ?? []).join(', ')));
  let busy = $state(false);
  let err = $state('');

  async function save(e: Event) {
    e.preventDefault();
    busy = true;
    err = '';
    const payload: ServerEntry = {
      ...$state.snapshot(f),
      tags: tagsStr
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean),
    };
    // Empty optional strings drop out (the field is optional; the mirror treats
    // an absent key as None via skip_serializing_if).
    for (const k of ['discord_url', 'website_url', 'motd_override', 'founded_at'] as const) {
      if (!payload[k]) payload[k] = undefined;
    }
    try {
      await api.saveServer(payload);
      onSaved();
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    } finally {
      busy = false;
    }
  }
</script>

<form class="panel editor" onsubmit={save}>
  <div class="hd">
    <h2 class="ttl">{isNew ? 'New server' : `Edit ${f.server_id}`}</h2>
    <div class="spacer"></div>
    <button type="button" onclick={onCancel}>Cancel</button>
    <button class="primary" type="submit" disabled={busy || !f.server_id || !f.pack_id}>
      {busy ? 'saving...' : 'Save'}
    </button>
  </div>
  {#if err}<div class="err mono">{err}</div>{/if}
  <div class="grid">
    <label>
      server_id
      <input bind:value={f.server_id} disabled={!isNew} placeholder="main" />
    </label>
    <label>
      pack_id
      <input bind:value={f.pack_id} list="packids" placeholder="Industrial" />
      <datalist id="packids">{#each packIds as p}<option value={p}></option>{/each}</datalist>
    </label>
    <label>display_name<input bind:value={f.display_name} /></label>
    <label>owner_display<input bind:value={f.owner_display} /></label>
    <label class="wide">tagline<input bind:value={f.tagline} /></label>
    <label class="wide">banner_url<input bind:value={f.banner_url} placeholder="https://..." /></label>
    <label class="wide">tags (comma-separated)<input bind:value={tagsStr} placeholder="tech, economy" /></label>
    <label>discord_url<input bind:value={f.discord_url} placeholder="https://discord.gg/..." /></label>
    <label>website_url<input bind:value={f.website_url} placeholder="https://..." /></label>
    <label class="wide">
      description_md
      <textarea rows="5" bind:value={f.description_md}></textarea>
    </label>
    <label class="chk"><input type="checkbox" bind:checked={f.featured} /> featured</label>
  </div>
</form>

<style>
  .editor {
    padding: 18px;
    margin-bottom: 18px;
  }
  .hd {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 14px;
  }
  .ttl {
    font-size: 15px;
  }
  .spacer {
    flex: 1;
  }
  .err {
    color: var(--danger);
    font-size: 12px;
    margin-bottom: 12px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px 16px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--fg-dim);
  }
  label.wide {
    grid-column: 1 / -1;
  }
  label.chk {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    color: var(--fg);
  }
</style>
