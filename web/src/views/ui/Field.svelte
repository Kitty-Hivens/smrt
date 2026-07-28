<script lang="ts" module>
  // Per-instance caption ids, so a control can point at the caption above it.
  let seq = 0;
  function nextId(): number {
    return ++seq;
  }
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  // A labeled field: human label on top, the caller's own control inside (so
  // two-way binding stays the caller's), optional hint below. The label is the
  // place to say what a raw config key means in plain words.
  //
  // A plain <div>, not a <label>: wrapping the control in a <label> made the
  // whole cell focus the input on click, which reads as the field grabbing focus
  // when you meant only to click near it. Focus now follows the control itself
  // (click it, or Tab), which is what the caption implies.
  //
  // That left the caption purely visual, though: a screen reader on the control
  // heard nothing, or heard a placeholder someone had echoed into an aria-label,
  // which names the example instead of the field and is worse than silence
  // (#55). The caption is named and the control points at it, so the two are one
  // field for everyone.
  let {
    label,
    hint,
    wide = false,
    children,
  }: {
    label: string;
    hint?: string;
    wide?: boolean;
    children: Snippet;
  } = $props();

  const captionId = `fld-${nextId()}`;

  // The control belongs to the caller, so the wiring happens here rather than as
  // an id threaded through every call site: one mechanism that cannot drift,
  // instead of forty chances to forget one. A control that already carries its
  // own name keeps it.
  function nameControl(node: HTMLElement) {
    const el = node.querySelector<HTMLElement>(
      'input, textarea, select, [role="combobox"]',
    );
    if (!el || el.getAttribute('aria-label') || el.getAttribute('aria-labelledby')) return;
    el.setAttribute('aria-labelledby', captionId);
  }
</script>

<div class="field" class:wide use:nameControl>
  <span class="lbl" id={captionId}>{label}</span>
  {@render children()}
  {#if hint}<span class="hint">{hint}</span>{/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .field.wide {
    grid-column: 1 / -1;
  }
  .lbl {
    font-size: var(--fs-sm);
    font-weight: 500;
    color: var(--fg);
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--fg-dim);
    line-height: 1.4;
  }
</style>
