<script lang="ts">
  import type { EdgeProps } from '@xyflow/svelte';
  import { clock, useClock } from '../lib/anim.svelte';

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
  let { id, sourceX, sourceY, targetX, targetY, data }: EdgeProps = $props();

  // one shared rAF for every tendril on screen; stops when the last one unmounts
  $effect(() => useClock());

  const color = $derived((data?.color as string) ?? 'var(--fg-dim)');
  // per-edge phase offset so the whole graph does not pulse in lockstep
  const phase = $derived((data?.phase as number) ?? 0);

  const d = $derived.by(() => {
    const t = clock.t + phase;
    const dx = targetX - sourceX;
    const dy = targetY - sourceY;
    const dist = Math.hypot(dx, dy) || 1;
    // a vertex every ~4px, as in the original (which steps every 2 at GUI scale)
    const n = Math.max(8, Math.min(160, Math.round(dist / 4)));
    const amp = 5;
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
<path class="glow" {d} stroke="url(#{gid})" />
<path class="core" {d} stroke="url(#{gid})" />

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
  }
  .glow {
    stroke-width: 6;
    opacity: 0.3;
    filter: blur(3px);
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
