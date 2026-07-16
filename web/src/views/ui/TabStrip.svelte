<script lang="ts">
  import { Tabs } from 'bits-ui';

  type Tab = { value: string; label: string; disabled?: boolean };

  let {
    value,
    tabs,
    variant = 'underline',
    ariaLabel,
    onChange,
  }: {
    value: string;
    tabs: Tab[];
    // underline: the flush seg strip; pill: the bordered pill group
    variant?: 'underline' | 'pill';
    ariaLabel?: string;
    onChange?: (value: string) => void;
  } = $props();
</script>

<Tabs.Root class="tabstrip-root" {value} onValueChange={(v) => onChange?.(v)}>
  <Tabs.List class={'tabstrip ' + variant} aria-label={ariaLabel}>
    {#each tabs as tb (tb.value)}
      <Tabs.Trigger value={tb.value} disabled={tb.disabled} class="tabstrip-trg">
        {tb.label}
      </Tabs.Trigger>
    {/each}
  </Tabs.List>
</Tabs.Root>

<style>
  /* Root/List/Trigger are Bits components, so classes on them get no Svelte
     scope hash -- these are global, uniquely named. Root is display:contents so
     the List is the caller's flex child, exactly where the old strip sat. This
     strip owns only the triggers; each caller keeps its own content, switched by
     the same value, so no Tabs.Content wrap is imposed on their layout. */
  :global(.tabstrip-root) {
    display: contents;
  }
  :global(.tabstrip) {
    display: flex;
  }
  :global(.tabstrip.underline) {
    gap: 2px;
  }
  :global(.tabstrip.pill) {
    display: inline-flex;
    gap: 2px;
    border: 1px solid var(--seam);
    border-radius: var(--radius-sm);
    padding: 2px;
    align-self: flex-start;
  }

  /* the trigger is a <button>, so the base button rules apply; reset to the
     bespoke strip look the native buttons carried. */
  :global(.tabstrip-trg) {
    box-shadow: none;
  }
  :global(.tabstrip.underline .tabstrip-trg) {
    background: transparent;
    border: 1px solid transparent;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    padding: 5px 12px;
    color: var(--fg-dim);
  }
  :global(.tabstrip.underline .tabstrip-trg[data-state='active']) {
    color: var(--fg);
    border-bottom-color: var(--accent);
  }
  :global(.tabstrip.pill .tabstrip-trg) {
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg-dim);
    font-size: 12px;
    padding: 5px 14px;
  }
  :global(.tabstrip.pill .tabstrip-trg:hover) {
    color: var(--fg);
  }
  :global(.tabstrip.pill .tabstrip-trg[data-state='active']) {
    background: var(--accent-soft);
    color: var(--accent-strong);
  }
  :global(.tabstrip-trg:disabled) {
    opacity: 0.45;
    cursor: default;
  }
</style>
