<script lang="ts">
  import GraphCanvas, { type EdgeFacts } from './GraphCanvas.svelte';
  import { api, ApiError } from '../lib/api';
  import { dialogs } from '../lib/dialogs.svelte';
  import { route } from '../lib/route.svelte';
  import { t } from '../lib/i18n.svelte';
  import { isDebug } from '../lib/roles';
  import type { GraphData, GraphSlice } from '../lib/types';

  // The registry-wide relation graph. This view owns which world is on show and
  // the debug write path; GraphCanvas owns the drawing and the focus.
  //
  // The registry holds every pack's mods at once, and mods from different
  // Minecraft versions never meet in one pack -- so the graph answers for one
  // (mc, loader) slice at a time (#49). Slice picks the world, focus picks the
  // neighbourhood inside it.
  let slices = $state<GraphSlice[]>([]);
  let mc = $state<string | null>(null);
  let loader = $state<string | null>(null);

  let raw = $state<GraphData | null>(null);
  let err = $state('');
  let loading = $state(true);
  let canDebug = $state(false);

  const KINDS = ['requires', 'optional_dep', 'recommends', 'conflicts', 'breaks', 'provides'];

  const fail = (e: unknown) => (e instanceof ApiError ? `${e.status} ${e.body}` : String(e));

  async function load() {
    loading = true;
    err = '';
    try {
      const [me, sl] = await Promise.all([api.me(), api.graphSlices()]);
      canDebug = isDebug(me?.role);
      slices = sl;
      // open on a world that has something in it -- the list arrives busiest first
      if (mc == null && loader == null && sl.length) {
        mc = sl[0].mc_version;
        loader = sl[0].loader;
      }
      raw = await api.graph(mc ?? undefined, loader ?? undefined);
    } catch (e) {
      err = fail(e);
    } finally {
      loading = false;
    }
  }
  load();

  // A real registry holds hundreds of (mc, loader) worlds -- one per Minecraft
  // version times every loader that has anything in it -- so a button per world is
  // a wall across the screen. Two dropdowns instead: Minecraft version, then the
  // loaders that version actually has. Both keep the busiest-first order the API
  // returns, so the populated worlds sit at the top rather than buried in a list.
  const mcOptions = $derived.by(() => {
    const order: string[] = [];
    const total = new Map<string, number>();
    for (const s of slices) {
      if (!total.has(s.mc_version)) order.push(s.mc_version);
      total.set(s.mc_version, (total.get(s.mc_version) ?? 0) + s.artifacts);
    }
    return order.map((mc_version) => ({ mc_version, artifacts: total.get(mc_version) ?? 0 }));
  });
  const loaderOptions = $derived(slices.filter((s) => s.mc_version === mc));

  async function loadSlice() {
    if (mc == null || loader == null) return;
    loading = true;
    err = '';
    try {
      raw = await api.graph(mc, loader);
    } catch (e) {
      err = fail(e);
    } finally {
      loading = false;
    }
  }

  function onMcChange() {
    // keep the current loader if this version has it, else jump to its busiest
    if (!loaderOptions.some((s) => s.loader === loader)) {
      loader = loaderOptions[0]?.loader ?? null;
    }
    void loadSlice();
  }

  async function pickKind(): Promise<string | null> {
    const raw_ = await dialogs.prompt(t('graph.kindPrompt', { kinds: KINDS.join(', ') }), {
      title: t('graph.addEdge'),
      initial: 'requires',
    });
    if (raw_ == null) return null;
    const kind = raw_.trim();
    if (!KINDS.includes(kind)) {
      err = t('graph.badKind');
      return null;
    }
    return kind;
  }

  // debug: connecting two mod nodes authors a relation (target by the mod's modid)
  async function onAuthorEdge(from: number, targetModid: string) {
    const kind = await pickKind();
    if (!kind) return;
    try {
      await api.authorRelation({ from_mod_id: from, target_modid: targetModid, kind });
      await load();
    } catch (e) {
      err = fail(e);
    }
  }

  // debug: deleting an authored edge removes the authored relation. Only authored
  // edges are deletable (a harvested fact would just reappear on re-harvest).
  async function onRemoveEdges(removed: EdgeFacts[]) {
    for (const d of removed) {
      if (!d?.authored || d.from == null || !d.target || !d.kind) continue;
      try {
        await api.authorRelation({
          from_mod_id: d.from,
          target_modid: d.target,
          kind: d.kind,
          remove: true,
        });
      } catch (e) {
        err = fail(e);
        await load(); // restore the view to the server truth on failure
      }
    }
  }
</script>

<div class="view">
  <div class="head">
    <span class="faint">{t('graph.hint')} {t('graph.clickHint')}</span>
    <div class="legend mono">
      <span class="lg" style="--c:var(--accent)">{t('graph.requires')}</span>
      <span class="lg" style="--c:var(--danger)">{t('graph.conflicts')}</span>
      <span class="lg" style="--c:var(--ok)">{t('graph.provides')}</span>
      <span class="lg" style="--c:var(--fg-dim)">{t('graph.optional')}</span>
    </div>
    <button class="sm" onclick={load} disabled={loading}>{t('graph.refresh')}</button>
  </div>
  {#if err}<div class="err mono">{err}</div>{/if}

  {#if slices.length > 1}
    <div class="slicebar">
      <span class="slabel mono">{t('graph.world')}</span>
      <select class="pick mono" bind:value={mc} onchange={onMcChange} aria-label={t('graph.mcVersion')}>
        {#each mcOptions as m (m.mc_version)}
          <option value={m.mc_version}>{m.mc_version} ({m.artifacts})</option>
        {/each}
      </select>
      <select class="pick mono" bind:value={loader} onchange={loadSlice} aria-label={t('graph.loader')}>
        {#each loaderOptions as s (s.loader)}
          <option value={s.loader}>{s.loader} ({s.artifacts})</option>
        {/each}
      </select>
    </div>
  {/if}

  {#if canDebug}<div class="dbg faint mono">{t('graph.debugHint')}</div>{/if}

  <GraphCanvas
    {raw}
    {loading}
    {canDebug}
    {onAuthorEdge}
    {onRemoveEdges}
    onError={(m) => (err = m)}
  >
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
  .dbg {
    font-size: 11px;
    color: var(--fg-dim);
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

  /* slice bar: which world the graph is answering for */
  .slicebar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .slabel {
    font-size: 10.5px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--fg-faint);
  }
  .slicebar .pick {
    width: auto;
    font-size: 12px;
    padding: 5px 10px;
  }
</style>
