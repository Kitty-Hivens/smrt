<script lang="ts">
  import GraphCanvas from './GraphCanvas.svelte';
  import { api, ApiError } from '../lib/api';
  import { route } from '../lib/route.svelte';
  import { t } from '../lib/i18n.svelte';
  import type { GraphData } from '../lib/types';

  // The pack's own relation graph: its mods, wired by what the exact artifacts it
  // ships declare. The registry-wide graph answers "what does the mirror hold";
  // this answers "does this pack hold together", which is the question actually
  // being asked while a pack is authored.
  //
  // Read-only: relations are facts about mods, not about this pack, so they are
  // authored in the registry's own graph rather than from inside an editor.
  let { packId }: { packId: string } = $props();

  let raw = $state<GraphData | null>(null);
  let loading = $state(true);
  let err = $state('');

  async function load() {
    loading = true;
    err = '';
    try {
      raw = await api.packGraph(packId);
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      loading = false;
    }
  }
  load();
</script>

<div class="view">
  <div class="head">
    <span class="faint">{t('pe.graphHint')}</span>
    <div class="legend mono">
      <span class="lg" style="--c:var(--accent)">{t('graph.requires')}</span>
      <span class="lg" style="--c:var(--danger)">{t('graph.conflicts')}</span>
      <span class="lg" style="--c:var(--ok)">{t('graph.provides')}</span>
      <span class="lg dashed">{t('pe.graphDangling')}</span>
    </div>
    <button class="sm" onclick={load} disabled={loading}>{t('graph.refresh')}</button>
  </div>
  {#if err}<div class="err mono">{err}</div>{/if}

  <GraphCanvas {raw} {loading} onError={(m) => (err = m)}>
    {#snippet actions(focusId: number)}
      <button class="sm" onclick={() => route.openMod(focusId)}>{t('graph.openPage')}</button>
    {/snippet}
  </GraphCanvas>
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3) var(--space-4);
    flex-wrap: wrap;
    font-size: 12px;
  }
  .legend {
    display: flex;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-left: auto;
  }
  .lg {
    font-size: 11px;
    color: var(--fg-dim);
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .lg::before {
    content: '';
    width: 14px;
    height: 2px;
    background: var(--c);
    display: inline-block;
  }
  /* a dangling target is a mod this pack does not carry -- the thing worth seeing */
  .lg.dashed::before {
    background: none;
    border-top: 1px dashed var(--fg-faint);
    height: 0;
  }
  .err {
    color: var(--danger);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    font-size: 12px;
  }
  button.sm {
    padding: 4px 10px;
    font-size: 12px;
  }
</style>
