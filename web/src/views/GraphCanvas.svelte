<script module lang="ts">
  /** what a removed edge asserted, so the owner can unauthor it */
  export type EdgeFacts = {
    authored?: boolean;
    from?: number;
    target?: string;
    kind?: string;
  };
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import {
    SvelteFlow,
    Background,
    Controls,
    type Node,
    type Edge,
    type Connection,
  } from '@xyflow/svelte';
  import '@xyflow/svelte/dist/style.css';
  import dagre from '@dagrejs/dagre';
  import TendrilEdge from './TendrilEdge.svelte';
  import GraphFit from './GraphFit.svelte';
  import TendrilLayer from './TendrilLayer.svelte';
  import { t } from '../lib/i18n.svelte';
  import { hover, drag } from '../lib/graphhover.svelte';
  import type { GraphData } from '../lib/types';

  // Draws a relation graph and lets you walk it. Owns the focus, the layout and
  // the canvas; it does not know where the data came from -- the registry-wide
  // view feeds it one (mc, loader) slice, a pack feeds it its own mods -- so both
  // get the same tendrils, the same focus, and the same reading of an edge.
  //
  // A whole graph at once is only readable while it is small: a real pack puts a
  // couple of hundred mods on the field, each requiring the same handful of
  // libraries, and the result is a hairball no styling can rescue. So this is
  // focus-first: pick a mod, see its neighbourhood, walk outwards by clicking.
  let {
    raw,
    loading = false,
    canDebug = false,
    onAuthorEdge,
    onRemoveEdges,
    onError,
    actions,
  }: {
    raw: GraphData | null;
    loading?: boolean;
    /** debug may draw an edge (authors a relation) or delete an authored one */
    canDebug?: boolean;
    onAuthorEdge?: (fromModId: number, targetModid: string) => void;
    onRemoveEdges?: (removed: EdgeFacts[]) => void;
    onError?: (message: string) => void;
    /** owner-supplied controls for the focused mod (e.g. open its page) */
    actions?: Snippet<[number]>;
  } = $props();

  let nodes = $state<Node[]>([]);
  let edges = $state<Edge[]>([]);

  // focus state: the mod at the centre, and how many relation hops out to draw
  let focusId = $state<number | null>(null);
  let hops = $state(1);
  // an explicit opt-in to render the whole web anyway, however unreadable
  let showAll = $state(false);
  let query = $state('');

  // Above this many mods the unfocused view is a hairball, so it is not drawn
  // until the operator either focuses something or explicitly asks for it.
  const BIG = 60;

  const focusedNode = $derived(raw?.nodes.find((n) => n.mod_id === focusId) ?? null);
  const needsFocus = $derived(
    !loading && raw !== null && raw.nodes.length > BIG && focusId === null && !showAll,
  );
  const empty = $derived(!loading && raw !== null && raw.nodes.length === 0);

  // node id -> its modid, to form the target selector when authoring an edge
  let modidById = new Map<string, string | undefined>();

  // edge stroke per relation kind
  const KIND_COLOR: Record<string, string> = {
    requires: 'var(--accent)',
    optional_dep: 'var(--fg-dim)',
    recommends: 'var(--fg-dim)',
    conflicts: 'var(--danger)',
    breaks: 'var(--danger)',
    provides: 'var(--ok)',
  };

  const edgeTypes = { tendril: TendrilEdge };

  const modNodeId = (modId: number) => `m${modId}`;
  const idToMod = (id: string) => (id.startsWith('m') ? parseInt(id.slice(1), 10) : NaN);

  /**
   * The mods within `depth` relation hops of `root`, walking edges in both
   * directions -- what a mod conflicts with matters as much as what it needs, and
   * so does whoever depends on it. External (uncatalogued) targets are not walked
   * through; they hang off whichever kept mod names them.
   */
  function neighborhood(g: GraphData, root: number, depth: number): Set<number> {
    const keep = new Set<number>([root]);
    let frontier = new Set<number>([root]);
    for (let h = 0; h < depth; h++) {
      const next = new Set<number>();
      for (const e of g.edges) {
        if (e.to_mod_id == null) continue;
        if (frontier.has(e.from_mod_id) && !keep.has(e.to_mod_id)) {
          keep.add(e.to_mod_id);
          next.add(e.to_mod_id);
        }
        if (frontier.has(e.to_mod_id) && !keep.has(e.from_mod_id)) {
          keep.add(e.from_mod_id);
          next.add(e.from_mod_id);
        }
      }
      frontier = next;
    }
    return keep;
  }

  /** Which mods to draw: the focus neighbourhood, or `null` for everything. */
  function visibleSet(g: GraphData): Set<number> | null {
    if (focusId != null) return neighborhood(g, focusId, hops);
    if (showAll || g.nodes.length <= BIG) return null;
    return new Set(); // big and unfocused: draw nothing, prompt for a focus instead
  }

  function build(g: GraphData, keep: Set<number> | null): { ns: Node[]; es: Edge[] } {
    modidById = new Map();
    const ns: Node[] = [];
    const seen = new Set<string>();
    const inScope = (modId: number) => keep === null || keep.has(modId);
    for (const n of g.nodes) {
      if (!inScope(n.mod_id)) continue;
      const id = modNodeId(n.mod_id);
      seen.add(id);
      modidById.set(id, n.modid ?? undefined);
      // the focused mod wears the panel's one filled emphasis (the inverted
      // white solid) -- no texture needed for it to read as the centre
      const base = n.mod_id === focusId ? 'gv-focus' : n.modrinth ? 'gv-modrinth' : 'gv-mod';
      ns.push({
        id,
        position: { x: 0, y: 0 },
        // `base` is kept so a hover can re-dress the node without a rebuild
        data: { label: n.name, base },
        class: base,
        connectable: canDebug,
        deletable: false,
      });
    }
    const es: Edge[] = [];
    g.edges.forEach((e, i) => {
      if (!inScope(e.from_mod_id)) return;
      if (e.to_mod_id != null && !inScope(e.to_mod_id)) return;
      const source = modNodeId(e.from_mod_id);
      let target: string;
      if (e.to_mod_id != null) {
        target = modNodeId(e.to_mod_id);
      } else {
        // An external / unresolved target: a labelled leaf, so the dangling
        // requirement stays visible. In a pack's graph this is the interesting
        // case -- it is a requirement the pack does not carry.
        //
        // While focused, only the focused mod's own external targets are drawn.
        // Every mod in a pack requires the same libraries, so letting each
        // neighbour bring its own external edges rebuilds the whole hairball
        // around a single leaf -- the focus would limit the mods and then undo
        // itself.
        if (keep !== null && e.from_mod_id !== focusId) return;
        target = `x:${e.target}`;
        if (!seen.has(target)) {
          seen.add(target);
          ns.push({
            id: target,
            position: { x: 0, y: 0 },
            data: { label: e.target, base: 'gv-ext' },
            class: 'gv-ext',
            connectable: false,
            deletable: false,
          });
        }
      }
      const authored = e.source === 'authored' || e.source === 'curator';
      const color = KIND_COLOR[e.kind] ?? 'var(--fg-dim)';
      // A hard incompatibility keeps the marching red dash -- it reads as an alarm,
      // which is exactly what it is. Everything else becomes a tendril: the colour
      // still carries the kind, and the travelling wave carries the direction.
      const alarm = e.kind === 'conflicts' || e.kind === 'breaks';
      // No `label`: the kind is already in the colour, and a label per edge turns a
      // real pack's graph into a field of plates.
      es.push({
        id: `e${i}`,
        source,
        target,
        type: alarm ? undefined : 'tendril',
        animated: alarm,
        selectable: true,
        deletable: canDebug && authored,
        style: alarm ? `stroke:${color};stroke-width:1.5` : undefined,
        data: {
          authored,
          kind: e.kind,
          target: e.target,
          from: e.from_mod_id,
          color,
          // stagger the wave so the graph does not pulse in lockstep
          phase: i * 13,
        },
      });
    });
    return { ns, es };
  }

  // left-to-right layered layout; dagre tolerates the cycles conflicts introduce
  function layout(ns: Node[], es: Edge[]) {
    const g = new dagre.graphlib.Graph();
    g.setGraph({ rankdir: 'LR', nodesep: 24, ranksep: 90 });
    g.setDefaultEdgeLabel(() => ({}));
    const W = 150;
    const H = 36;
    for (const n of ns) g.setNode(n.id, { width: W, height: H });
    for (const e of es) g.setEdge(e.source, e.target);
    dagre.layout(g);
    for (const n of ns) {
      const p = g.node(n.id);
      n.position = { x: p.x - W / 2, y: p.y - H / 2 };
    }
  }

  // bumped on every rebuild so the camera re-frames the new layout
  let fitToken = $state(0);

  /** Re-derive what is on screen from `raw` + the current focus, and lay it out. */
  function rebuild() {
    // the node under the cursor is about to stop existing; a stale hover would
    // leave the whole graph muted against a mod that is no longer drawn
    hover.set(null);
    const g = raw;
    if (!g) {
      nodes = [];
      edges = [];
      return;
    }
    const { ns, es } = build(g, visibleSet(g));
    layout(ns, es);
    nodes = ns;
    edges = es;
    fitToken++;
  }

  // a fresh graph (a new slice, a reloaded pack) resets the focus: the old centre
  // may not exist here at all
  let lastRaw: GraphData | null = null;
  $effect(() => {
    if (raw === lastRaw) return;
    lastRaw = raw;
    if (focusId != null && !raw?.nodes.some((n) => n.mod_id === focusId)) focusId = null;
    rebuild();
  });

  function setFocus(modId: number) {
    focusId = modId;
    rebuild();
  }
  function setHops(n: number) {
    hops = n;
    rebuild();
  }
  // Leave focus, but keep the "show the whole web" opt-in: if the operator chose
  // to reveal everything and then focused a mod, clearing focus returns them to
  // the whole web, not back to the too-big prompt they already dismissed.
  function clearFocus() {
    focusId = null;
    query = '';
    rebuild();
  }
  // The explicit way back to the prompt from the revealed whole web.
  function collapseAll() {
    showAll = false;
    focusId = null;
    query = '';
    rebuild();
  }
  function showEverything() {
    showAll = true;
    focusId = null;
    rebuild();
  }

  // the picker: exact name first, then the first substring hit
  function applyQuery() {
    const g = raw;
    const q = query.trim().toLowerCase();
    if (!g || !q) return;
    const hit =
      g.nodes.find((n) => n.name.toLowerCase() === q) ??
      g.nodes.find((n) => n.name.toLowerCase().includes(q));
    if (hit) setFocus(hit.mod_id);
    else onError?.(t('graph.noSuchMod'));
  }

  // Clicking a mod node re-centres the focus on it, so the graph is walked rather
  // than left: navigating away on every click would lose your place. The focused
  // mod's page is one explicit button away in the focus bar. External/unresolved
  // leaves carry no mod id and are not focusable.
  function onnodeclick({ node }: { node: Node }) {
    const modId = idToMod(node.id);
    if (Number.isFinite(modId)) setFocus(modId);
  }

  // Hovering a mod lights its own path and lets everything else recede. The edges
  // read the hovered id straight from the shared store, so only the nodes are
  // re-dressed here -- and only their class, never their position, so the layout
  // is not re-run under the cursor.
  function markHover(nodeId: string | null) {
    hover.set(nodeId);
    const near = new Set<string>();
    if (nodeId != null) {
      near.add(nodeId);
      for (const e of edges) {
        if (e.source === nodeId) near.add(e.target);
        if (e.target === nodeId) near.add(e.source);
      }
    }
    nodes = nodes.map((n) => {
      const base = (n.data as { base?: string }).base ?? 'gv-mod';
      return { ...n, class: nodeId == null || near.has(n.id) ? base : `${base} gv-mute` };
    });
  }
  const onnodepointerenter = ({ node }: { node: Node }) => markHover(node.id);
  const onnodepointerleave = () => markHover(null);

  function onconnect(conn: Connection) {
    if (!canDebug) return;
    const from = idToMod(conn.source);
    const targetModid = modidById.get(conn.target);
    if (!Number.isFinite(from) || !targetModid) {
      onError?.(t('graph.needModid'));
      return;
    }
    onAuthorEdge?.(from, targetModid);
  }

  function ondelete({ edges: removed }: { edges: Edge[] }) {
    onRemoveEdges?.(removed.map((e) => e.data as EdgeFacts));
  }
</script>

<div class="focusbar">
  <input
    class="pick mono"
    list="gv-mods"
    bind:value={query}
    placeholder={t('graph.focusPlaceholder')} aria-label={t('graph.focusPlaceholder')}
    onchange={applyQuery}
    onkeydown={(e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        applyQuery();
      }
    }}
  />
  <datalist id="gv-mods">
    {#each raw?.nodes ?? [] as n (n.mod_id)}<option value={n.name}></option>{/each}
  </datalist>

  {#if focusId != null}
    <span class="fname">{focusedNode?.name}</span>
    <div class="hops" role="group" aria-label={t('graph.hops')}>
      {#each [1, 2] as h}
        <button class="hop" class:active={hops === h} aria-pressed={hops === h} onclick={() => setHops(h)}>
          {h}
        </button>
      {/each}
    </div>
    <span class="count faint mono">{t('graph.showingN', { n: nodes.length })}</span>
    {@render actions?.(focusId)}
    <button class="sm" onclick={clearFocus}>{t('graph.clearFocus')}</button>
  {:else if showAll}
    <span class="count faint mono">{t('graph.showingN', { n: nodes.length })}</span>
    <button class="sm" onclick={collapseAll}>{t('graph.collapse')}</button>
  {/if}
</div>

<div class="flowwrap">
  {#if empty}
    <div class="empty muted">{t('graph.empty')}</div>
  {:else if needsFocus}
    <div class="empty prompt">
      <div class="ptext muted">{t('graph.tooBig', { n: raw?.nodes.length ?? 0 })}</div>
      <button class="sm" onclick={showEverything}>{t('graph.showAll')}</button>
    </div>
  {:else}
    <SvelteFlow
      bind:nodes
      bind:edges
      {edgeTypes}
      fitView
      nodesConnectable={canDebug}
      deleteKey={['Delete', 'Backspace']}
      {onnodeclick}
      {onnodepointerenter}
      {onnodepointerleave}
      onnodedragstart={() => drag.set(true)}
      onnodedragstop={() => drag.set(false)}
      {onconnect}
      {ondelete}
    >
      <Background />
      <TendrilLayer />
      <Controls />
      <GraphFit token={fitToken} />
    </SvelteFlow>
  {/if}
</div>

<style>
  /* focus bar */
  .focusbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .pick {
    width: 260px;
    font-size: var(--fs-sm);
    padding: 6px 10px;
  }
  .fname {
    font-size: var(--fs-md);
    font-weight: 600;
    margin-left: var(--space-2);
  }
  .hops {
    display: inline-flex;
    border: 1px solid var(--seam-bright);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }
  .hop {
    border: none;
    border-radius: 0;
    padding: 4px 10px;
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--fg-dim);
    background: transparent;
  }
  .hop:hover {
    background: var(--panel-2);
  }
  .hop.active {
    background: var(--accent-soft);
    color: var(--accent-strong);
  }
  .count {
    font-size: var(--fs-xs);
  }
  button.sm {
    padding: 4px 10px;
    font-size: var(--fs-sm);
  }

  .flowwrap {
    position: relative;
    height: calc(100vh - 260px);
    min-height: 420px;
    border: 1px solid var(--seam);
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--panel);
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: var(--fs-md);
  }
  .prompt {
    align-content: center;
    gap: var(--space-4);
    text-align: center;
    padding: var(--space-5);
  }
  .ptext {
    max-width: 46ch;
    line-height: 1.6;
  }

  /* node chrome, themed with the panel tokens rather than Svelte Flow defaults */
  .flowwrap :global(.svelte-flow__node) {
    font-size: var(--fs-xs);
    font-family: var(--mono);
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--seam-bright);
    background: var(--panel-2);
    color: var(--fg);
    width: 150px;
    text-align: center;
  }
  .flowwrap :global(.svelte-flow__node.gv-modrinth) {
    border-color: color-mix(in srgb, var(--info) 55%, var(--seam));
  }
  /* the focused mod: the panel's inverted solid, the same emphasis its one primary
     button uses. Reads as the centre instantly on this field, with no texture to
     light up the way a hand-drawn research web would. */
  .flowwrap :global(.svelte-flow__node.gv-focus) {
    background: var(--solid);
    border-color: var(--solid);
    color: var(--on-solid);
    font-weight: 700;
  }
  .flowwrap :global(.svelte-flow__node.gv-ext) {
    border-style: dashed;
    color: var(--fg-dim);
    background: transparent;
  }
  /* not on the hovered mod's path: recede, so what is left standing is exactly
     what that mod touches */
  .flowwrap :global(.svelte-flow__node.gv-mute) {
    opacity: 0.12;
  }
  .flowwrap :global(.svelte-flow__node) {
    transition: opacity var(--dur-state) var(--ease-out);
  }
  @media (prefers-reduced-motion: reduce) {
    .flowwrap :global(.svelte-flow__node) {
      transition: none;
    }
  }

  /* zoom / fit / lock controls: the library ships them white, which on this field
     reads as four blank plates. Dress them like every other control here. */
  .flowwrap :global(.svelte-flow__controls) {
    box-shadow: none;
  }
  .flowwrap :global(.svelte-flow__controls-button) {
    width: 26px;
    height: 26px;
    padding: 5px;
    background: var(--panel-2);
    border: 1px solid var(--seam);
    border-bottom: none;
    color: var(--fg-dim);
    fill: currentColor;
  }
  .flowwrap :global(.svelte-flow__controls-button:first-child) {
    border-radius: var(--radius-sm) var(--radius-sm) 0 0;
  }
  .flowwrap :global(.svelte-flow__controls-button:last-child) {
    border-bottom: 1px solid var(--seam);
    border-radius: 0 0 var(--radius-sm) var(--radius-sm);
  }
  .flowwrap :global(.svelte-flow__controls-button:hover) {
    background: var(--panel-3);
    color: var(--fg);
  }
  .flowwrap :global(.svelte-flow__controls-button svg) {
    fill: currentColor;
    max-width: 12px;
    max-height: 12px;
  }
  /* the library's attribution stays (its licence asks for it) -- just stop it
     glowing white against the field */
  .flowwrap :global(.svelte-flow__attribution) {
    background: transparent;
  }
  .flowwrap :global(.svelte-flow__attribution a) {
    color: var(--fg-faint);
  }
</style>
