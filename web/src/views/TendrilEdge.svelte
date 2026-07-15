<script lang="ts">
  import { useSvelteFlow, type EdgeProps } from '@xyflow/svelte';
  import { clock, useClock } from '../lib/anim.svelte';
  import { hover } from '../lib/graphhover.svelte';

  // A living relation line, ported from the Thaumcraft 4 Thaumonomicon's research
  // links. The original draws a GL line strip sampled every couple of pixels and
  // displaces each vertex by two sines of different period driven by the game's
  // tick counter, so the wave travels along the line instead of sitting still.
  //
  // Two envelopes carry the meaning, and both are kept here:
  //   * the wiggle is scaled by (1 - p), so the line thrashes where it leaves the
  //     source and settles calm as it enters the target -- energy flows one way,
  //     which is what tells you the direction without an arrowhead;
  //   * the original's additive brightness works out to colour*(1-p) * 0.6*p, a
  //     product that peaks mid-span and reaches zero at both ends, so the tendril
  //     floats between the two nodes rather than touching them. The gradient below
  //     traces that same parabola (4p(1-p)), and `screen` blending stands in for
  //     the GL additive blend so crossings brighten the way they do in-game.
  let { id, source, target, sourceX, sourceY, targetX, targetY, data }: EdgeProps = $props();

  // one shared rAF for every tendril on screen; stops when the last one unmounts
  $effect(() => useClock());

  // On a mod's path, or nothing is hovered at all. An unlit tendril stops
  // travelling and drops to a whisper: on a hub's fan of fifty, the one path you
  // are pointing at is the only thing still alive, which is the whole point --
  // and a still tendril is a straight line, so it costs almost nothing to draw.
  const lit = $derived(hover.id == null || source === hover.id || target === hover.id);

  // The wave is sampled per screen pixel, not per graph unit. Zoomed out to fit a
  // whole registry, an edge 300 units long is twenty pixels of screen -- sampling
  // it by graph length spent seventy-odd vertices drawing a wave finer than a
  // pixel, on every edge, every frame. Nobody could see it and everybody paid for
  // it. At reading zoom this changes nothing.
  const { getViewport } = useSvelteFlow();

  const color = $derived((data?.color as string) ?? 'var(--fg-dim)');
  // per-edge phase offset so the whole graph does not pulse in lockstep
  const phase = $derived((data?.phase as number) ?? 0);

  const d = $derived.by(() => {
    const t = clock.t + phase;
    const dx = targetX - sourceX;
    const dy = targetY - sourceY;
    const dist = Math.hypot(dx, dy) || 1;
    // a vertex every ~4 screen px, as in the original (which steps every 2 at GUI
    // scale); an unlit tendril is a straight line and needs none
    const onScreen = dist * getViewport().zoom;
    const n = lit ? Math.max(6, Math.min(160, Math.round(onScreen / 4))) : 1;
    const amp = lit ? 5 : 0;
    let out = '';
    for (let i = 0; i <= n; i++) {
      const p = i / n; // 0 at the source, 1 at the target
      const env = 1 - p; // the wiggle dies out as it reaches the target
      const x = sourceX + dx * p + Math.sin((t + i) / 7) * amp * env;
      const y = sourceY + dy * p + Math.sin((t + i) / 5) * amp * env;
      out += `${i === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`;
    }
    return out;
  });

  const gid = $derived(`tendril-${id}`);
</script>

<defs>
  <linearGradient
    id={gid}
    gradientUnits="userSpaceOnUse"
    x1={sourceX}
    y1={sourceY}
    x2={targetX}
    y2={targetY}
  >
    <!-- 4p(1-p): nothing at either node, full brightness mid-span -->
    <stop offset="0%" stop-color={color} stop-opacity="0" />
    <stop offset="25%" stop-color={color} stop-opacity="0.75" />
    <stop offset="50%" stop-color={color} stop-opacity="1" />
    <stop offset="75%" stop-color={color} stop-opacity="0.75" />
    <stop offset="100%" stop-color={color} stop-opacity="0" />
  </linearGradient>
</defs>

<!-- invisible fat stroke so the edge stays clickable (debug selects it to delete) -->
<path class="hit" {d} />
<path class="glow" class:unlit={!lit} {d} stroke="url(#{gid})" />
<path class="core" class:unlit={!lit} class:hot={lit && hover.id != null} {d} stroke="url(#{gid})" />

<style>
  path {
    fill: none;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .hit {
    stroke: transparent;
    stroke-width: 14;
    pointer-events: stroke;
  }
  .glow,
  .core {
    pointer-events: none;
    /* stands in for the original's additive blend: overlapping tendrils brighten */
    mix-blend-mode: screen;
    transition: opacity 0.18s ease;
  }
  /* Off the hovered mod's path: recede to a whisper so the live path is the only
     thing left to read. These go low: `screen` blending adds, so on a hub's fan
     fifty faint lines converging on one node still pile up into a glow. */
  .glow.unlit {
    opacity: 0;
  }
  .core.unlit {
    opacity: 0.05;
  }
  /* the path under the cursor, with the rest receding: give it a little more body
     so it reads as the one live thing rather than merely the last one standing */
  .core.hot {
    stroke-width: 2.4;
  }
  @media (prefers-reduced-motion: reduce) {
    .glow,
    .core {
      transition: none;
    }
  }
  /* The halo is a wide soft stroke, not a blur filter. A `filter: blur()` on a
     path whose geometry changes every frame makes the browser re-rasterize it
     every frame, per edge: on a hub's fan of fifty that measured 50ms of a 66ms
     frame -- the entire cost of the view. Grouping the blur into one pass is not
     available either, since Svelte Flow gives every edge its own <g>. A wide,
     low-opacity stroke under the bright core reads as the same halo on this field
     and costs nothing to move. */
  .glow {
    stroke-width: 7;
    opacity: 0.22;
  }
  .core {
    stroke-width: 1.6;
  }
  /* a selected edge (debug is about to delete it) burns brighter */
  :global(.svelte-flow__edge.selected) .core {
    stroke-width: 2.6;
  }
  :global(.svelte-flow__edge.selected) .glow {
    opacity: 0.55;
  }
</style>
