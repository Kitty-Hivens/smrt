<script lang="ts">
  import GraphCanvas from './GraphCanvas.svelte';
  import { api, ApiError } from '../lib/api';
  import { notifyFail, toasts } from '../lib/toasts.svelte';
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

  async function load() {
    loading = true;
    try {
      raw = await api.packGraph(packId);
    } catch (e) {
      notifyFail(e);
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

  <GraphCanvas {raw} {loading} onError={(m) => toasts.push({ kind: 'error', text: m })}>
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
    font-size: var(--fs-sm);
  }
  .legend {
    display: flex;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-left: auto;
  }
  .lg {
    font-size: var(--fs-xs);
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
  button.sm {
    padding: 4px 10px;
    font-size: var(--fs-sm);
  }
</style>
