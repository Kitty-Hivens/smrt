<script lang="ts">
  import type { Snippet } from 'svelte';

  // A titled card: heading (with optional count) + an actions slot on the right
  // + body. The default surface for grouping related controls.
  let {
    title,
    count,
    actions,
    children,
    flush = false,
  }: {
    title: string;
    count?: number;
    actions?: Snippet;
    children: Snippet;
    flush?: boolean; // body has no padding (e.g. wraps a table)
  } = $props();
</script>

<section class="section panel">
  <header class="head">
    <h3 class="ttl">
      {title}{#if count !== undefined}<span class="count">{count}</span>{/if}
    </h3>
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </header>
  <div class="body" class:flush>
    {@render children()}
  </div>
</section>

<style>
  .section {
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--seam);
    background: var(--panel-2);
  }
  .ttl {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--fg-dim);
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }
  .count {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--fg-faint);
    text-transform: none;
    letter-spacing: 0;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .body {
    padding: var(--space-4);
  }
  .body.flush {
    padding: 0;
  }
</style>
