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
</script>

<div class="field" class:wide>
  <span class="lbl">{label}</span>
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
