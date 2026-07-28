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
    error,
    wide = false,
    children,
  }: {
    label: string;
    hint?: string;
    /// What is wrong with the value in this field, said at the field while it is
    /// being typed rather than as a notice after a save the server refused.
    error?: string | null;
    wide?: boolean;
    children: Snippet;
  } = $props();

  const captionId = `fld-${nextId()}`;
  const errorId = `${captionId}-err`;

  // The control belongs to the caller, so the wiring happens here rather than as
  // an id threaded through every call site: one mechanism that cannot drift,
  // instead of forty chances to forget one. A control that already carries its
  // own name keeps it.
  function control(node: HTMLElement): HTMLElement | null {
    return node.querySelector<HTMLElement>('input, textarea, select, [role="combobox"]');
  }

  function nameControl(node: HTMLElement) {
    const el = control(node);
    if (!el) return;
    if (!el.getAttribute('aria-label') && !el.getAttribute('aria-labelledby')) {
      el.setAttribute('aria-labelledby', captionId);
    }
    // The error has to reach the control, not just the eye: a caption below it
    // that nothing points at is the same defect the label had.
    $effect(() => {
      if (error) {
        el.setAttribute('aria-invalid', 'true');
        el.setAttribute('aria-describedby', errorId);
      } else {
        el.removeAttribute('aria-invalid');
        el.removeAttribute('aria-describedby');
      }
    });
  }
</script>

<div class="field" class:wide use:nameControl>
  <span class="lbl" id={captionId}>{label}</span>
  {@render children()}
  {#if error}<span class="err" id={errorId}>{error}</span>{/if}
  {#if hint && !error}<span class="hint">{hint}</span>{/if}
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
  /* the error takes the hint's place rather than pushing it down: a field that
     grows while being typed in moves everything under it */
  .err {
    font-size: var(--fs-xs);
    color: var(--danger);
    line-height: 1.4;
  }
  .field:has(.err) :global(input),
  .field:has(.err) :global(textarea) {
    border-color: var(--danger);
  }
</style>
