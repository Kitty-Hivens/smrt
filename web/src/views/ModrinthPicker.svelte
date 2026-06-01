<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import type { ModrinthHit } from '../lib/types';

  let {
    mc,
    onPick,
    onClose,
  }: {
    mc?: string;
    onPick: (sel: { project_id: string; slug: string; version_id: string; title: string }) => void;
    onClose: () => void;
  } = $props();

  let q = $state('');
  let hits = $state<ModrinthHit[]>([]);
  let busy = $state(false);
  let err = $state('');
  let resolving = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;

  function onInput() {
    clearTimeout(timer);
    timer = setTimeout(search, 300);
  }

  async function search() {
    if (!q.trim()) {
      hits = [];
      return;
    }
    busy = true;
    err = '';
    try {
      hits = await api.modrinthSearch(q.trim(), mc);
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      busy = false;
    }
  }

  async function pick(h: ModrinthHit) {
    resolving = h.project_id;
    err = '';
    try {
      const vers = await api.modrinthVersions(h.slug, mc);
      if (vers.length === 0) {
        err = `No ${mc ?? ''} versions for ${h.title}`;
        return;
      }
      onPick({ project_id: h.project_id, slug: h.slug, version_id: vers[0].id, title: h.title });
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      resolving = '';
    }
  }
</script>

<div class="overlay" onclick={onClose} role="presentation">
  <div class="picker panel" onclick={(e) => e.stopPropagation()} role="presentation">
    <div class="ph row">
      <input bind:value={q} oninput={onInput} placeholder={`Search Modrinth${mc ? ` (${mc})` : ''}...`} />
      <button onclick={onClose}>Close</button>
    </div>
    {#if err}<div class="err mono">{err}</div>{/if}
    {#if busy}<div class="muted s">searching...</div>{/if}
    <div class="hits scroll">
      {#each hits as h}
        <button class="hit" onclick={() => pick(h)} disabled={resolving === h.project_id}>
          {#if h.icon_url}<img src={h.icon_url} alt="" />{:else}<div class="ic" ></div>{/if}
          <div class="info">
            <div class="t">{h.title} <span class="faint mono">{h.slug}</span></div>
            <div class="d muted">{h.description}</div>
          </div>
          {#if resolving === h.project_id}<span class="muted mono rs">resolving...</span>{/if}
        </button>
      {/each}
      {#if hits.length === 0 && q.trim() && !busy}<div class="muted s">No results.</div>{/if}
    </div>
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
    width: 620px;
    max-width: 92vw;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    padding: 16px;
  }
  .ph {
    gap: 10px;
    margin-bottom: 10px;
  }
  .err {
    color: var(--danger);
    font-size: 12px;
    margin-bottom: 8px;
  }
  .s {
    font-size: 12px;
    padding: 6px 0;
  }
  .hits {
    overflow: auto;
  }
  .hit {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    text-align: left;
    padding: 8px;
    border: 1px solid transparent;
    border-bottom: 1px solid var(--seam);
    background: transparent;
  }
  .hit:hover {
    background: var(--panel-2);
    border-color: transparent;
    border-bottom-color: var(--seam);
  }
  .hit img,
  .hit .ic {
    width: 38px;
    height: 38px;
    object-fit: cover;
    background: var(--bg);
    border: 1px solid var(--seam);
    flex-shrink: 0;
  }
  .info {
    flex: 1;
    min-width: 0;
  }
  .t {
    font-size: 13px;
  }
  .d {
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rs {
    font-size: 11px;
  }
</style>
