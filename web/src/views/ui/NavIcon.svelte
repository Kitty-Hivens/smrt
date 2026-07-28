<script lang="ts">
  import type { Section } from '../../lib/route.svelte';

  // One leading glyph per rail section. Inline SVG rather than an icon font or
  // a sprite fetched at runtime: the rail is the first thing painted, and a
  // glyph that arrives late reads as a layout bug. `currentColor` throughout,
  // so a section inherits the same dim/active colour the label already has.
  let { name, size = 15 }: { name: Section; size?: number } = $props();
</script>

<svg
  class="ico"
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.3"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === 'browse'}
    <rect x="2.4" y="2.4" width="4.8" height="4.8" rx="1" />
    <rect x="8.8" y="2.4" width="4.8" height="4.8" rx="1" />
    <rect x="2.4" y="8.8" width="4.8" height="4.8" rx="1" />
    <rect x="8.8" y="8.8" width="4.8" height="4.8" rx="1" />
  {:else if name === 'overview'}
    <path d="M2.4 13.4h11.2" />
    <path d="M4.6 13.4V9.4" />
    <path d="M8 13.4V5.2" />
    <path d="M11.4 13.4V7.6" />
  {:else if name === 'packs'}
    <path d="M8 2.2l5.6 3v5.6L8 13.8l-5.6-3V5.2z" />
    <path d="M2.4 5.2L8 8.2l5.6-3" />
    <path d="M8 8.2v5.6" />
  {:else if name === 'mypacks'}
    <path d="M8 2.8l4.6 2.5v4.8L8 12.6l-4.6-2.5V5.3z" />
    <path d="M3.4 5.3L8 7.8l4.6-2.5" />
    <path d="M8 7.8v4.8" />
    <circle cx="13.2" cy="3" r="1.6" fill="currentColor" stroke="none" />
  {:else if name === 'servers'}
    <rect x="2.4" y="3" width="11.2" height="4.2" rx="1.2" />
    <rect x="2.4" y="8.8" width="11.2" height="4.2" rx="1.2" />
    <circle cx="5" cy="5.1" r="0.8" fill="currentColor" stroke="none" />
    <circle cx="5" cy="10.9" r="0.8" fill="currentColor" stroke="none" />
  {:else if name === 'mods'}
    <path d="M8 1.9l5.9 3.1L8 8.1 2.1 5z" />
    <path d="M2.1 8.4L8 11.5l5.9-3.1" />
    <path d="M2.1 11.3L8 14.4l5.9-3.1" />
  {:else if name === 'graph'}
    <path d="M5.2 5.4l5.6-0.7" />
    <path d="M4.6 6.6l3.1 4.7" />
    <path d="M11.2 6.3l-2.4 5" />
    <circle cx="3.8" cy="4.6" r="1.7" />
    <circle cx="12.2" cy="4.6" r="1.7" />
    <circle cx="8.2" cy="12.4" r="1.7" />
  {:else if name === 'users'}
    <circle cx="6.4" cy="5.6" r="2.4" />
    <path d="M1.9 13.6c0-2.6 2-3.9 4.5-3.9s4.5 1.3 4.5 3.9" />
    <path d="M11.4 4.1a2.1 2.1 0 010 4" />
    <path d="M12.4 10.1c1.2.5 1.9 1.6 1.9 3.5" />
  {:else if name === 'moderation'}
    <path d="M8 2l5.4 2.1v3.8c0 3.3-2.9 5.1-5.4 6.1-2.5-1-5.4-2.8-5.4-6.1V4.1z" />
    <path d="M5.9 8.1l1.5 1.5 2.9-3" />
  {:else if name === 'audit'}
    <path d="M6.2 4.4h7.4" />
    <path d="M6.2 8h7.4" />
    <path d="M6.2 11.6h7.4" />
    <circle cx="3.2" cy="4.4" r="0.9" fill="currentColor" stroke="none" />
    <circle cx="3.2" cy="8" r="0.9" fill="currentColor" stroke="none" />
    <circle cx="3.2" cy="11.6" r="0.9" fill="currentColor" stroke="none" />
  {:else if name === 'settings'}
    <path d="M2.6 4.4h4" />
    <path d="M9.4 4.4h4" />
    <path d="M2.6 11.6h4.8" />
    <path d="M10.2 11.6h3.2" />
    <circle cx="7.8" cy="4.4" r="1.7" />
    <circle cx="8.9" cy="11.6" r="1.7" />
  {:else if name === 'profile'}
    <circle cx="8" cy="5.4" r="2.6" />
    <path d="M2.9 13.8c0-2.9 2.3-4.3 5.1-4.3s5.1 1.4 5.1 4.3" />
  {/if}
</svg>

<style>
  .ico {
    flex: none;
    /* the rail's labels are uppercase mono; a glyph sitting on the same optical
       line as their cap-height reads as part of the word, not above it */
    margin-top: -1px;
  }
</style>
