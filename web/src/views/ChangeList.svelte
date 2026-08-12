<script lang="ts">
  // What separates two states of a pack, as rows a person reads.
  //
  // One list, three places: what a commit is about to record, what a commit did
  // record, and what a restore would put back. The rows come from the mirror --
  // the same diff the build gate counts -- so the number beside a list and the
  // list itself cannot disagree, which is exactly what they used to do.
  import { t } from '../lib/i18n.svelte';
  import type { ChangeGroup, ConfigChange } from '../lib/types';

  let {
    rows,
    labels = {},
    dense = false,
  }: {
    rows: ConfigChange[];
    /// Modrinth `version_id -> version_number`, filled in by whoever mounts this.
    /// A pin on screen has to read like a version: only Modrinth knows what
    /// `bqMxf6Ua` is called, and the row is readable either way.
    labels?: Record<string, string>;
    /// Capped height with its own scroll, for the commit box; a page shows the
    /// whole list.
    dense?: boolean;
  } = $props();

  const GROUPS: ChangeGroup[] = ['pack', 'mods', 'assets'];

  const grouped = $derived(
    GROUPS.map((group) => ({ group, rows: rows.filter((r) => r.group === group) })).filter(
      (g) => g.rows.length,
    ),
  );

  const counts = $derived({
    add: rows.filter((r) => r.op === 'add').length,
    remove: rows.filter((r) => r.op === 'remove').length,
    change: rows.filter((r) => r.op === 'change').length,
  });

  // Groups start open. A curator who added forty mods wants to see that they
  // are the forty they meant, and folding by default hides exactly the case
  // where reading matters most.
  let folded = $state<Record<string, boolean>>({});

  /// A pin as a person reads it: the version number where Modrinth has been
  /// asked, and a short hash otherwise. A jar the mirror knows only by content
  /// has 40 hex characters for a name, and printing both ends of a re-pin in
  /// full says nothing while filling the row.
  const label = (pin?: string) => {
    if (!pin) return '';
    const known = labels[pin];
    if (known) return known;
    return /^[0-9a-f]{40}$/.test(pin) ? pin.slice(0, 8) : pin;
  };

  const sign = (op: string) => (op === 'add' ? '+' : op === 'remove' ? '-' : '~');
</script>

{#if rows.length}
  <div class="changes" class:dense>
    <div class="tally">
      {#if counts.add}<span class="add" title={t('chg.added')}>+{counts.add}</span>{/if}
      {#if counts.remove}<span class="remove" title={t('chg.removed')}>-{counts.remove}</span>{/if}
      {#if counts.change}<span class="change" title={t('chg.changed')}>~{counts.change}</span>{/if}
    </div>

    {#each grouped as g (g.group)}
      <section>
        <button
          class="head"
          onclick={() => (folded = { ...folded, [g.group]: !folded[g.group] })}
          aria-expanded={!folded[g.group]}
        >
          <span class="caret" class:folded={folded[g.group]}>&rsaquo;</span>
          <span class="name">{t(`chg.group.${g.group}`)}</span>
          <span class="n">{g.rows.length}</span>
        </button>
        {#if !folded[g.group]}
          <ul>
            {#each g.rows as r (r.group + r.op + r.key + (r.field ?? ''))}
              <li class={r.op}>
                <span class="sign">{sign(r.op)}</span>
                <span class="what mono">{r.label}</span>
                {#if r.op === 'change' && r.field === 'display'}
                  <span class="move muted">{t('chg.field.display')}</span>
                {:else if r.op === 'change' && r.from !== undefined && r.to !== undefined}
                  {#if r.field && r.field !== 'value'}
                    <span class="aspect muted">{t(`chg.field.${r.field}`)}</span>
                  {/if}
                  <span class="move mono" title="{r.from} &rarr; {r.to}"
                    >{label(r.from)} &rarr; {label(r.to)}</span
                  >
                {:else if r.op === 'change'}
                  <span class="move muted">{t('chg.edited')}</span>
                {:else if r.to !== undefined && r.to !== r.label}
                  <span class="move mono" title={r.to}>{label(r.to)}</span>
                {:else if r.from !== undefined && r.from !== r.label}
                  <!-- a static asset pins its own path: printing it beside the
                       path it is already named by says the same thing twice -->
                  <span class="move mono" title={r.from}>{label(r.from)}</span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/each}
  </div>
{/if}

<style>
  .changes {
    font-size: var(--fs-sm);
  }
  .dense ul {
    max-height: 190px;
    overflow-y: auto;
  }
  .tally {
    display: flex;
    gap: 10px;
    margin-bottom: 6px;
    font-variant-numeric: tabular-nums;
  }
  .tally .add {
    color: var(--ok, var(--fg));
  }
  .tally .remove {
    color: var(--danger, var(--fg));
  }
  .tally .change {
    color: var(--fg-dim);
  }
  section {
    margin-bottom: 6px;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 7px;
    width: 100%;
    background: none;
    border: 0;
    padding: 2px 0;
    font: inherit;
    color: var(--fg-dim);
    cursor: pointer;
    text-align: left;
  }
  .head:hover .name {
    color: var(--fg);
  }
  .caret {
    display: inline-block;
    transition: transform var(--dur-fast, 120ms) var(--ease-out, ease);
    transform: rotate(90deg);
  }
  .caret.folded {
    transform: rotate(0deg);
  }
  .n {
    font-variant-numeric: tabular-nums;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0 0 0 14px;
  }
  li {
    display: flex;
    gap: 8px;
    align-items: baseline;
    padding: 1px 0;
  }
  .sign {
    width: 1ch;
    color: var(--fg-dim);
  }
  li.add .sign {
    color: var(--ok, var(--fg));
  }
  li.remove .sign {
    color: var(--danger, var(--fg));
  }
  .what {
    overflow-wrap: anywhere;
  }
  .aspect,
  .move {
    color: var(--fg-dim);
  }
</style>
