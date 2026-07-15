<script lang="ts">
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
  import { api, ApiError } from '../lib/api';
  import { dialogs } from '../lib/dialogs.svelte';
  import { route } from '../lib/route.svelte';
  import { t } from '../lib/i18n.svelte';
  import { isDebug } from '../lib/roles';
  import type { GraphData } from '../lib/types';

  // The dependency/conflict graph. Read-only for an operator; a debug user can
  // draw an edge (authors a relation) or delete an authored one (#33 phase 3-4).
  let nodes = $state<Node[]>([]);
  let edges = $state<Edge[]>([]);
  let err = $state('');
  let loading = $state(true);
  let canDebug = $state(false);
  let empty = $state(false);

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
  const KINDS = ['requires', 'optional_dep', 'recommends', 'conflicts', 'breaks', 'provides'];

  const modNodeId = (modId: number) => `m${modId}`;
  const idToMod = (id: string) => (id.startsWith('m') ? parseInt(id.slice(1), 10) : NaN);

  function build(g: GraphData): { ns: Node[]; es: Edge[] } {
    modidById = new Map();
    const ns: Node[] = [];
    const seen = new Set<string>();
    for (const n of g.nodes) {
      const id = modNodeId(n.mod_id);
      seen.add(id);
      modidById.set(id, n.modid ?? undefined);
      ns.push({
        id,
        position: { x: 0, y: 0 },
        data: { label: n.name },
        class: n.modrinth ? 'gv-modrinth' : 'gv-mod',
        connectable: canDebug,
        deletable: false,
      });
    }
    const es: Edge[] = [];
    g.edges.forEach((e, i) => {
      const source = modNodeId(e.from_mod_id);
      let target: string;
      if (e.to_mod_id != null) {
        target = modNodeId(e.to_mod_id);
      } else {
        // an external / unresolved target (uncatalogued modid or a capability):
        // render it as a labelled leaf so the dangling edge stays visible
        target = `x:${e.target}`;
        if (!seen.has(target)) {
          seen.add(target);
          ns.push({
            id: target,
            position: { x: 0, y: 0 },
            data: { label: e.target },
            class: 'gv-ext',
            connectable: false,
            deletable: false,
          });
        }
      }
      const authored = e.source === 'authored' || e.source === 'curator';
      es.push({
        id: `e${i}`,
        source,
        target,
        label: e.kind,
        animated: e.kind === 'conflicts' || e.kind === 'breaks',
        selectable: true,
        deletable: canDebug && authored,
        style: `stroke:${KIND_COLOR[e.kind] ?? 'var(--fg-dim)'};stroke-width:1.5`,
        data: { authored, kind: e.kind, target: e.target, from: e.from_mod_id },
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

  async function load() {
    loading = true;
    err = '';
    try {
      const [me, g] = await Promise.all([api.me(), api.graph()]);
      canDebug = isDebug(me?.role);
      empty = g.nodes.length === 0;
      const { ns, es } = build(g);
      layout(ns, es);
      nodes = ns;
      edges = es;
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    } finally {
      loading = false;
    }
  }
  load();

  async function pickKind(): Promise<string | null> {
    const raw = await dialogs.prompt(t('graph.kindPrompt', { kinds: KINDS.join(', ') }), {
      title: t('graph.addEdge'),
      initial: 'requires',
    });
    if (raw == null) return null;
    const kind = raw.trim();
    if (!KINDS.includes(kind)) {
      err = t('graph.badKind');
      return null;
    }
    return kind;
  }

  // clicking a mod node opens its page; external/unresolved leaves have no id
  function onnodeclick({ node }: { node: Node }) {
    const modId = idToMod(node.id);
    if (Number.isFinite(modId)) route.openMod(modId);
  }

  // debug: connecting two mod nodes authors a relation (target by the mod's modid)
  async function onconnect(conn: Connection) {
    if (!canDebug) return;
    const from = idToMod(conn.source);
    const targetModid = modidById.get(conn.target);
    if (!Number.isFinite(from) || !targetModid) {
      err = t('graph.needModid');
      return;
    }
    const kind = await pickKind();
    if (!kind) return;
    try {
      await api.authorRelation({ from_mod_id: from, target_modid: targetModid, kind });
      await load();
    } catch (e) {
      err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
    }
  }

  // debug: deleting an authored edge removes the authored relation. Only authored
  // edges are deletable (a harvested fact would just reappear on re-harvest).
  async function ondelete({ edges: removed }: { edges: Edge[] }) {
    for (const e of removed) {
      const d = e.data as { authored?: boolean; from?: number; target?: string; kind?: string };
      if (!d?.authored || d.from == null || !d.target || !d.kind) continue;
      try {
        await api.authorRelation({
          from_mod_id: d.from,
          target_modid: d.target,
          kind: d.kind,
          remove: true,
        });
      } catch (e2) {
        err = e2 instanceof ApiError ? `${e2.status} ${e2.body}` : String(e2);
        await load(); // restore the view to the server truth on failure
      }
    }
  }
</script>

<div class="view">
  <div class="head">
    <span class="faint">{t('graph.hint')}</span>
    <div class="legend mono">
      <span class="lg" style="--c:var(--accent)">{t('graph.requires')}</span>
      <span class="lg" style="--c:var(--danger)">{t('graph.conflicts')}</span>
      <span class="lg" style="--c:var(--ok)">{t('graph.provides')}</span>
      <span class="lg" style="--c:var(--fg-dim)">{t('graph.optional')}</span>
    </div>
    <button class="sm" onclick={load} disabled={loading}>{t('graph.refresh')}</button>
  </div>
  {#if err}<div class="err mono">{err}</div>{/if}
  {#if canDebug}<div class="dbg faint mono">{t('graph.debugHint')}</div>{/if}

  <div class="flowwrap">
    {#if empty && !loading}
      <div class="empty muted">{t('graph.empty')}</div>
    {:else}
      <SvelteFlow
        bind:nodes
        bind:edges
        fitView
        nodesConnectable={canDebug}
        deleteKey={['Delete', 'Backspace']}
        {onnodeclick}
        {onconnect}
        {ondelete}
      >
        <Background />
        <Controls />
      </SvelteFlow>
    {/if}
  </div>
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
  .flowwrap {
    position: relative;
    height: calc(100vh - 220px);
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
    font-size: 13px;
  }
  button.sm {
    padding: 4px 10px;
    font-size: 12px;
  }

  /* node chrome, themed with the panel tokens rather than Svelte Flow defaults */
  .flowwrap :global(.svelte-flow__node) {
    font-size: 11px;
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
  .flowwrap :global(.svelte-flow__node.gv-ext) {
    border-style: dashed;
    color: var(--fg-dim);
    background: transparent;
  }
  .flowwrap :global(.svelte-flow__edge-text) {
    font-size: 9px;
    fill: var(--fg-dim);
  }
  .flowwrap :global(.svelte-flow__edge-textbg) {
    fill: var(--panel);
  }
</style>
