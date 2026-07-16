<script lang="ts">
  import { useStore } from '@xyflow/svelte';
  import { clock, useClock } from '../lib/anim.svelte';
  import { hover } from '../lib/graphhover.svelte';
  import { tendrils } from '../lib/tendrils.svelte';

  // Paints every tendril on one canvas, under the nodes.
  //
  // This is what makes the effect survive a real pack. It is also closer to the
  // original than the SVG paths were: the Thaumonomicon draws its links with an
  // additive blend, which canvas has outright (`lighter`) where SVG only had
  // `mix-blend-mode: screen` standing in for it.
  //
  // Sits at z-index 1: above the background (-1) and below the viewport (2), which
  // is where the nodes live. The edges' own SVG stays for hit-testing -- a static
  // straight line that never animates, so it costs nothing.
  const store = useStore();
  $effect(() => useClock());

  let canvas = $state<HTMLCanvasElement | null>(null);
  let box = $state({ w: 0, h: 0 });

  // The spindle gradient depends only on where the edge's ends sit on screen, so
  // it survives every frame nothing moved -- which is most of them. Rebuilding
  // five hundred per frame was pure waste.
  //
  // Keyed by those ends, not by the edge: dragging a node moves an edge without
  // touching the viewport, and a gradient cached against the old position would
  // leave the spindle anchored where the edge used to be.
  const gradients = new Map<string, CanvasGradient>();

  // Kind -> colour, resolved once from the panel's tokens. Canvas cannot read
  // `var(--accent)`, and the spindle needs an alpha ramp along the line, so the
  // tokens are resolved all the way to channels and rebuilt as rgba stops.
  type Rgb = [number, number, number];
  let palette = $state<Record<string, Rgb>>({});

  function hexToRgb(hex: string, fallback: Rgb): Rgb {
    const h = hex.trim().replace('#', '');
    const full = h.length === 3 ? [...h].map((c) => c + c).join('') : h;
    if (full.length < 6) return fallback;
    const n = Number.parseInt(full.slice(0, 6), 16);
    return Number.isNaN(n) ? fallback : [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }

  function resolvePalette(el: HTMLElement) {
    const cs = getComputedStyle(el);
    const v = (name: string, fallback: Rgb): Rgb =>
      hexToRgb(cs.getPropertyValue(name), fallback);
    const dim = v('--fg-dim', [154, 154, 158]);
    const danger = v('--danger', [240, 87, 106]);
    palette = {
      requires: v('--accent', [255, 255, 255]),
      optional_dep: dim,
      recommends: dim,
      conflicts: danger,
      breaks: danger,
      provides: v('--ok', [78, 203, 139]),
      _default: dim,
    };
  }

  $effect(() => {
    const el = canvas;
    if (!el) return;
    resolvePalette(el);
    const parent = el.parentElement;
    if (!parent) return;
    const ro = new ResizeObserver(() => {
      const r = parent.getBoundingClientRect();
      box = { w: r.width, h: r.height };
    });
    ro.observe(parent);
    const r = parent.getBoundingClientRect();
    box = { w: r.width, h: r.height };
    return () => ro.disconnect();
  });

  // Redraw every frame: the wave travels, so there is nothing to cache. Reading
  // the clock here is what schedules it.
  $effect(() => {
    const el = canvas;
    const { w, h } = box;
    const t = clock.t;
    const vp = store.viewport;
    const hovered = hover.id;
    if (!el || w === 0 || h === 0) return;

    const dpr = window.devicePixelRatio || 1;
    if (el.width !== Math.round(w * dpr) || el.height !== Math.round(h * dpr)) {
      el.width = Math.round(w * dpr);
      el.height = Math.round(h * dpr);
    }
    const ctx = el.getContext('2d');
    if (!ctx) return;

    // bound so a long pan cannot grow it without limit
    if (gradients.size > 4000) gradients.clear();

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    // the original blends additively; crossings brighten instead of covering
    ctx.globalCompositeOperation = 'lighter';
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';

    for (const [, e] of tendrils.all()) {
      const lit = hovered == null || e.source === hovered || e.target === hovered;
      const [r, g, bl] = palette[e.kind] ?? palette._default ?? [154, 154, 158];

      // to screen space, so a wave is never sampled finer than a pixel
      const sx = e.sx * vp.zoom + vp.x;
      const sy = e.sy * vp.zoom + vp.y;
      const tx = e.tx * vp.zoom + vp.x;
      const ty = e.ty * vp.zoom + vp.y;
      // skip what is off-screen entirely
      const pad = 40;
      if (
        (sx < -pad && tx < -pad) ||
        (sx > w + pad && tx > w + pad) ||
        (sy < -pad && ty < -pad) ||
        (sy > h + pad && ty > h + pad)
      ) {
        continue;
      }

      const dx = tx - sx;
      const dy = ty - sy;
      const dist = Math.hypot(dx, dy) || 1;
      const n = lit ? Math.max(6, Math.min(140, Math.round(dist / 4))) : 1;
      const amp = lit ? 5 * vp.zoom : 0;
      const time = t + e.phase;

      ctx.beginPath();
      for (let i = 0; i <= n; i++) {
        const p = i / n;
        const env = 1 - p; // the wiggle dies out as it enters the target
        const x = sx + dx * p + Math.sin((time + i) / 7) * amp * env;
        const y = sy + dy * p + Math.sin((time + i) / 5) * amp * env;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }

      // 4p(1-p): nothing at either node, full brightness mid-span -- the original's
      // additive product of a fading colour and a rising alpha
      const gk = `${e.kind}|${sx.toFixed(0)},${sy.toFixed(0)},${tx.toFixed(0)},${ty.toFixed(0)}`;
      let grad = gradients.get(gk);
      if (!grad) {
        grad = ctx.createLinearGradient(sx, sy, tx, ty);
        for (const [at, a] of [
          [0, 0],
          [0.25, 0.75],
          [0.5, 1],
          [0.75, 0.75],
          [1, 0],
        ] as const) {
          grad.addColorStop(at, `rgba(${r},${g},${bl},${a})`);
        }
        gradients.set(gk, grad);
      }
      ctx.strokeStyle = grad;

      // The halo is a wide stroke and the most expensive thing here. It earns its
      // cost only when there is a path to make pop -- while hovering. Idle, five
      // hundred overlapping haloes just smear into a glow the additive core gives
      // anyway, so the whole pass is skipped and the draw roughly halves.
      if (hovered != null && lit) {
        ctx.globalAlpha = 0.16;
        ctx.lineWidth = 7;
        ctx.stroke();
      }
      ctx.globalAlpha = lit ? 1 : 0.05;
      ctx.lineWidth = lit && hovered != null ? 2.4 : 1.6;
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
    ctx.globalCompositeOperation = 'source-over';
  });
</script>

<canvas
  bind:this={canvas}
  class="tendrils svelte-flow__container"
  style="width:{box.w}px;height:{box.h}px"
  aria-hidden="true"
></canvas>

<style>
  .tendrils {
    z-index: 1;
    pointer-events: none;
  }
</style>
