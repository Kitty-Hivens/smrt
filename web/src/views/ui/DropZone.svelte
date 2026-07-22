<script lang="ts">
  import type { Snippet } from 'svelte';

  // Drag files in or click to pick. The caller gets File[] and decides what to
  // do (upload, add, etc.). Keyboard-openable.
  let {
    onFiles,
    accept,
    multiple = true,
    label,
    busy = false,
    children,
  }: {
    onFiles: (files: File[]) => void;
    accept?: string;
    multiple?: boolean;
    label: string;
    busy?: boolean;
    children?: Snippet;
  } = $props();

  let over = $state(false);
  let input = $state<HTMLInputElement | null>(null);

  function emit(files: FileList | null | undefined) {
    const list = [...(files ?? [])];
    if (list.length) onFiles(list);
  }
</script>

<div
  class="dz"
  class:over
  class:busy
  role="button"
  tabindex="0"
  aria-label={label}
  onclick={() => input?.click()}
  onkeydown={(e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      input?.click();
    }
  }}
  ondragover={(e) => {
    e.preventDefault();
    over = true;
  }}
  ondragleave={() => (over = false)}
  ondrop={(e) => {
    e.preventDefault();
    over = false;
    emit(e.dataTransfer?.files);
  }}
>
  <input
    bind:this={input}
    type="file"
    {accept}
    {multiple}
    hidden
    onchange={(e) => {
      emit((e.target as HTMLInputElement).files);
      (e.target as HTMLInputElement).value = '';
    }}
  />
  {#if children}{@render children()}{:else}<span class="lbl">{label}</span>{/if}
</div>

<style>
  .dz {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: 10px var(--space-4);
    border: 1px dashed var(--seam-bright);
    border-radius: var(--radius-md);
    background: var(--bg-2);
    color: var(--fg-dim);
    font-size: 12.5px;
    cursor: pointer;
    text-align: center;
    transition:
      border-color 0.12s ease,
      background 0.12s ease,
      color 0.12s ease;
  }
  .dz:hover {
    border-color: var(--seam-bright);
    color: var(--fg);
  }
  .dz:focus-visible {
    outline: none;
    border-color: var(--accent);
    box-shadow: var(--ring);
  }
  .dz.over {
    border-color: var(--accent);
    border-style: solid;
    background: var(--accent-soft);
    color: var(--accent-strong);
  }
  .dz.busy {
    opacity: 0.6;
    pointer-events: none;
  }
</style>
