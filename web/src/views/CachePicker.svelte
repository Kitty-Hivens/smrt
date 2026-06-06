<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import type { CacheUsageEntry } from '../lib/types';

  let {
    onPick,
    onClose,
  }: {
    onPick: (sel: { sha1: string; filename: string }) => void;
    onClose: () => void;
  } = $props();

  let entries = $state<CacheUsageEntry[]>([]);
  let q = $state('');
  let loading = $state(true);
  let err = $state('');

  async function load() {
    loading = true;
    err = '';
    try {
      entries = (await api.cacheUsage()).entries;
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
    loading = false;
  }
  load();

  // a content-addressed jar has no stored name; show the filename a pack gave it
  const nameOf = (e: CacheUsageEntry) => e.uses[0]?.filename ?? '';
  const usedByOf = (e: CacheUsageEntry) => [...new Set(e.uses.map((u) => u.pack_id))];

  const shown = $derived(
    entries.filter((e) => {
      const needle = q.trim().toLowerCase();
      if (!needle) return true;
      return e.sha1.includes(needle) || nameOf(e).toLowerCase().includes(needle);
    }),
  );

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

  function pick(e: CacheUsageEntry) {
    onPick({ sha1: e.sha1, filename: nameOf(e) || `${e.sha1.slice(0, 12)}.jar` });
  }
</script>

<div class="overlay" onclick={onClose} role="presentation">
  <div class="picker panel" onclick={(e) => e.stopPropagation()} role="presentation">
    <div class="ph row">
      <input bind:value={q} placeholder={t('cachePick.search')} />
      <button onclick={onClose}>{t('common.close')}</button>
    </div>
    {#if err}<div class="err mono">{err}</div>{/if}
    {#if loading}<div class="muted s">{t('common.loading')}</div>{/if}
    <div class="hits scroll">
      {#each shown as e (e.sha1)}
        <button class="hit" onclick={() => pick(e)}>
          <div class="info">
            <div class="t">
              {nameOf(e) || t('cachePick.noName')}
              {#if e.uses.length === 0}<span class="tag orphan">{t('cachePick.orphan')}</span>{/if}
            </div>
            <div class="d muted mono">
              {e.sha1.slice(0, 16)} · {fmtBytes(e.size_bytes)}
              {#if usedByOf(e).length}· {usedByOf(e).join(', ')}{/if}
            </div>
          </div>
        </button>
      {/each}
      {#if shown.length === 0 && !loading}
        <div class="muted s">{q.trim() ? t('cachePick.noMatch') : t('cachePick.empty')}</div>
      {/if}
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
    padding: var(--space-4);
  }
  .ph {
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }
  .err {
    color: var(--danger);
    font-size: 12px;
    margin-bottom: var(--space-2);
  }
  .s {
    font-size: 12px;
    padding: var(--space-2) 0;
  }
  .hits {
    overflow: auto;
  }
  .hit {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    text-align: left;
    padding: var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    border-bottom: 1px solid var(--seam);
    background: transparent;
  }
  .hit:hover {
    background: var(--panel-2);
  }
  .info {
    flex: 1;
    min-width: 0;
  }
  .t {
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .d {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin-top: 2px;
  }
  .tag.orphan {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 45%, transparent);
  }
</style>
