<script lang="ts">
  import { Select } from 'bits-ui';

  type Option = { value: string; label: string; disabled?: boolean };

  let {
    value = $bindable(''),
    options,
    placeholder = '',
    disabled = false,
    compact = false,
    full = false,
    ariaLabel,
    title,
    onChange,
  }: {
    value?: string;
    options: Option[];
    placeholder?: string;
    disabled?: boolean;
    // sm sizing (filter / inline selects) vs the default form-field size
    compact?: boolean;
    // stretch the trigger to its container instead of fitting its content
    full?: boolean;
    ariaLabel?: string;
    title?: string;
    onChange?: (value: string) => void;
  } = $props();

  const selectedLabel = $derived(options.find((o) => o.value === value)?.label ?? '');
</script>

<Select.Root
  type="single"
  bind:value
  {disabled}
  items={options}
  onValueChange={(v) => onChange?.(v)}
>
  <Select.Trigger
    class={'sel-trg' + (compact ? ' sm' : '') + (full ? ' full' : '')}
    aria-label={ariaLabel}
    {title}
  >
    <span class="sel-val" class:ph={!selectedLabel}>{selectedLabel || placeholder}</span>
    <svg class="sel-chev" width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
      <path
        d="M2.5 4 L5 6.5 L7.5 4"
        fill="none"
        stroke="currentColor"
        stroke-width="1.3"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  </Select.Trigger>
  <Select.Portal>
    <Select.Content class="sel-content" sideOffset={4}>
      <Select.Viewport class="sel-vp">
        {#each options as opt (opt.value)}
          <Select.Item class="sel-item" value={opt.value} label={opt.label} disabled={opt.disabled}>
            {#snippet children({ selected })}
              <span class="sel-item-lbl">{opt.label}</span>
              {#if selected}
                <svg class="sel-ck" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                  <path
                    d="M2.5 6.5 L5 9 L9.5 3.5"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  />
                </svg>
              {/if}
            {/snippet}
          </Select.Item>
        {/each}
      </Select.Viewport>
    </Select.Content>
  </Select.Portal>
</Select.Root>

<style>
  /* Every part rides a Bits component, so a class passed to it gets no Svelte
     scope hash -- these are global. Names are unique to this primitive. The
     trigger is a <button>, so the base button rules apply; here it is reset to
     read as a field (native <select> look), not a raised button. */
  :global(.sel-trg) {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    width: fit-content;
    max-width: 100%;
    font-family: var(--mono);
    font-weight: 400;
    font-size: var(--fs-md);
    padding: 9px 12px;
    box-shadow: none;
    text-align: left;
  }
  :global(.sel-trg.full) {
    width: 100%;
  }
  :global(.sel-trg.sm) {
    font-size: var(--fs-sm);
    padding: 5px 10px;
  }
  :global(.sel-trg:hover) {
    /* a field, not a button: hold the surface, let the border carry the state */
    background: var(--panel-2);
    border-color: var(--fg-faint);
  }
  :global(.sel-trg[data-state='open']) {
    border-color: var(--fg);
  }
  :global(.sel-trg[data-disabled]) {
    opacity: 0.45;
    cursor: default;
  }
  :global(.sel-val) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  :global(.sel-val.ph) {
    color: var(--fg-faint);
  }
  :global(.sel-chev) {
    flex-shrink: 0;
    color: var(--fg-dim);
    transition: transform var(--dur-state) var(--ease-out);
  }
  :global(.sel-trg[data-state='open'] .sel-chev) {
    transform: rotate(180deg);
  }

  /* dropdown -- portalled to <body>, so it must clear the dialog layer (z 61) */
  :global(.sel-content) {
    z-index: 90;
    min-width: var(--bits-floating-anchor-width);
    max-width: 92vw;
    background: var(--panel);
    border: 1px solid var(--seam-bright);
    border-radius: var(--radius-sm);
    padding: 4px;
  }
  :global(.sel-vp) {
    max-height: 264px;
    overflow-y: auto;
  }
  :global(.sel-item) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: 6px 8px;
    border-radius: var(--radius-sm);
    font-family: var(--mono);
    font-size: var(--fs-sm);
    color: var(--fg-dim);
    cursor: pointer;
    user-select: none;
  }
  :global(.sel-item[data-highlighted]) {
    background: var(--panel-2);
    color: var(--fg);
  }
  :global(.sel-item[data-selected]) {
    color: var(--fg);
  }
  :global(.sel-item[data-disabled]) {
    opacity: 0.4;
    pointer-events: none;
  }
  :global(.sel-item-lbl) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  :global(.sel-ck) {
    flex-shrink: 0;
    color: var(--fg);
  }
</style>
