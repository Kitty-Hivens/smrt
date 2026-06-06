<script lang="ts">
  import type { Snippet } from 'svelte';

  // A labeled field: human label on top, the caller's own control inside (so
  // two-way binding stays the caller's), optional hint below. The label is the
  // place to say what a raw config key means in plain words.
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

<label class="field" class:wide>
  <span class="lbl">{label}</span>
  {@render children()}
  {#if hint}<span class="hint">{hint}</span>{/if}
</label>

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
    font-size: 12px;
    font-weight: 500;
    color: var(--fg);
  }
  .hint {
    font-size: 11.5px;
    color: var(--fg-dim);
    line-height: 1.4;
  }
</style>
