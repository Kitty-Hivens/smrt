<script lang="ts">
  import type { Snippet } from 'svelte';
  import { t } from '../../lib/i18n.svelte';

  // A panel that lives above the page instead of inside it.
  //
  // Reports used to be inserted at the top of the editor, so asking for one
  // pushed the form down by however tall the answer happened to be -- the
  // content moved as a side effect of reading it. A dock overlays instead: the
  // editor underneath never reflows, and the operator parks the panel wherever
  // it does not cover what they are working on.
  //
  // `id` keys the remembered position, so a dock returns where it was left.

  let {
    id,
    title,
    subtitle,
    width = 460,
    onClose,
    header,
    children,
  }: {
    id: string;
    title: string;
    subtitle?: string;
    width?: number;
    onClose: () => void;
    /// Extra controls rendered in the title bar (tabs, actions).
    header?: Snippet;
    children: Snippet;
  } = $props();

  const MARGIN = 8;
  // one storage slot per dock instance; `id` is fixed for the life of a dock
  const storageKey = $derived(`smrt.dock.${id}`);

  function stored(): { x: number; y: number } | null {
    try {
      const raw = localStorage.getItem(storageKey);
      if (!raw) return null;
      const p = JSON.parse(raw);
      return typeof p?.x === 'number' && typeof p?.y === 'number' ? p : null;
    } catch {
      return null;
    }
  }

  // Default berth: top-right, clear of the shell's top bar. Right-aligned
  // because the editor's own controls sit left, so the dock lands where the
  // form is not.
  function initial(): { x: number; y: number } {
    return stored() ?? { x: Math.max(MARGIN, window.innerWidth - width - 24), y: 88 };
  }

  let pos = $state(initial());
  let dragging = $state(false);
  let panel = $state<HTMLElement | null>(null);

  function clamp(p: { x: number; y: number }) {
    const w = panel?.offsetWidth ?? width;
    return {
      // keep a grabbable strip on screen in both axes, whatever the viewport does
      x: Math.min(Math.max(MARGIN - w + 80, p.x), window.innerWidth - 80),
      y: Math.min(Math.max(MARGIN, p.y), window.innerHeight - 40),
    };
  }

  let grab = { dx: 0, dy: 0 };

  function onGrab(e: PointerEvent) {
    // the title bar carries buttons; only bare bar surface starts a drag
    if ((e.target as HTMLElement).closest('button')) return;
    dragging = true;
    grab = { dx: e.clientX - pos.x, dy: e.clientY - pos.y };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onMove(e: PointerEvent) {
    if (!dragging) return;
    pos = clamp({ x: e.clientX - grab.dx, y: e.clientY - grab.dy });
  }

  function onDrop() {
    if (!dragging) return;
    dragging = false;
    try {
      localStorage.setItem(storageKey, JSON.stringify(pos));
    } catch {
      // private mode / blocked storage: the dock just forgets where it was
    }
  }

  // a viewport that shrinks must not strand the panel off-screen
  $effect(() => {
    const onResize = () => (pos = clamp(pos));
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') onClose();
  }
</script>

<svelte:window onkeydown={onKey} />

<!-- enters as one object: a short rise and fade, no scale bounce. The dock is a
     panel being placed, not a notification popping -->
<section
  class="dock enter"
  class:dragging
  bind:this={panel}
  style="left:{pos.x}px; top:{pos.y}px; width:{width}px"
  aria-label={title}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -- the bar is a drag
       handle for a pointer; keyboard users move nothing and lose nothing, and
       every control inside it is a real button -->
  <header class="bar" onpointerdown={onGrab} onpointermove={onMove} onpointerup={onDrop} onpointercancel={onDrop}>
    <div class="titles">
      <span class="ttl">{title}</span>
      {#if subtitle}<span class="sub faint">{subtitle}</span>{/if}
    </div>
    {#if header}{@render header()}{/if}
    <button class="x" onclick={onClose} aria-label={t('common.close')}>×</button>
  </header>
  <div class="content">
    {@render children()}
  </div>
</section>

<style>
  @keyframes dock-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
  .dock.enter {
    animation: dock-in var(--dur-layer) var(--ease-out) backwards;
  }
  .dock {
    position: fixed;
    z-index: 55;
    max-width: calc(100vw - 16px);
    max-height: min(70vh, 720px);
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border: 1px solid var(--seam-bright);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-pop);
    overflow: hidden;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--seam);
    background: var(--panel-2);
    cursor: grab;
    touch-action: none;
    user-select: none;
  }
  .dock.dragging .bar {
    cursor: grabbing;
  }
  .titles {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    min-width: 0;
    flex: 1;
  }
  .ttl {
    font-size: var(--fs-sm);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .sub {
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .x {
    border: none;
    background: transparent;
    color: var(--fg-faint);
    font-size: var(--fs-xl);
    line-height: 1;
    padding: 2px 4px;
    cursor: pointer;
  }
  .x:hover {
    color: var(--fg);
  }
  .content {
    overflow: auto;
    padding: var(--space-3);
  }
  /* A draggable panel is a pointer affordance; on a phone it becomes a sheet
     pinned to the bottom, where a thumb can reach it and nothing can strand it. */
  @media (max-width: 560px) {
    .dock {
      left: 0 !important;
      top: auto !important;
      bottom: 0;
      width: 100% !important;
      max-width: 100%;
      max-height: 60vh;
      border-radius: var(--radius-md) var(--radius-md) 0 0;
    }
    .bar {
      cursor: default;
    }
  }
</style>
