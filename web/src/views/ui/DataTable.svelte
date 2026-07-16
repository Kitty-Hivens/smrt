<script lang="ts" module>
  import {
    createTable,
    tableFeatures,
    rowSortingFeature,
    createSortedRowModel,
    sortFn_alphanumeric,
    columnFilteringFeature,
    globalFilteringFeature,
    createFilteredRowModel,
    filterFn_includesString,
  } from '@tanstack/svelte-table';

  // Stable across the module: row sorting and a global filter, and only those.
  // Global filtering builds on the column-filtering feature, so it is included
  // too. The table owns every slice internally -- callers drive sort through the
  // header handlers and the filter through the controlled `filter` prop.
  const FEATURES = tableFeatures({
    rowSortingFeature,
    sortedRowModel: createSortedRowModel(),
    sortFns: { alphanumeric: sortFn_alphanumeric },
    columnFilteringFeature,
    globalFilteringFeature,
    filteredRowModel: createFilteredRowModel(),
    filterFns: { includesString: filterFn_includesString },
  });

  export type Column<T> = {
    id: string;
    header: string;
    width?: string;
    sortable?: boolean;
    // sort + global-filter accessor; columns without one (actions, icon-only)
    // neither sort nor match a filter
    value?: (row: T) => string | number;
  };
</script>

<script lang="ts" generics="T extends RowData">
  import type { Snippet } from 'svelte';
  import type { RowData } from '@tanstack/svelte-table';

  let {
    data,
    columns,
    row,
    empty,
    filter = '',
  }: {
    data: T[];
    columns: Column<T>[];
    // renders one full <tr>...</tr> for row.original
    row: Snippet<[T]>;
    // rendered as the tbody body when there are no rows (a full-width <tr>)
    empty?: Snippet;
    // caller-owned global filter text; the caller renders its own input. Empty
    // string (the default) filters nothing, so a table with no filter box is
    // simply left uncontrolled.
    filter?: string;
  } = $props();

  const tsColumns = $derived(
    columns.map((c) => ({
      id: c.id,
      header: c.header,
      accessorFn: c.value ?? (() => ''),
      enableSorting: c.sortable ?? false,
      enableGlobalFilter: c.value != null,
      sortingFn: 'alphanumeric' as const,
      filterFn: 'includesString' as const,
    })),
  );

  // column widths live on the caller's spec, not in TanStack column meta, so the
  // header can size a <th> without a ColumnMeta module augmentation
  const widthById = $derived(new Map(columns.map((c) => [c.id, c.width])));

  const table = createTable({
    features: FEATURES,
    get data() {
      return data;
    },
    get columns() {
      return tsColumns;
    },
    globalFilterFn: 'includesString',
    get state() {
      return { globalFilter: filter };
    },
  });

  const ariaSort = (s: false | 'asc' | 'desc') =>
    s === 'asc' ? 'ascending' : s === 'desc' ? 'descending' : 'none';
</script>

<div class="tablewrap">
  <table>
    <thead>
      {#each table.getHeaderGroups() as hg (hg.id)}
        <tr>
          {#each hg.headers as header (header.id)}
            {@const sorted = header.column.getIsSorted()}
            <th
              style={widthById.get(header.column.id)
                ? `width:${widthById.get(header.column.id)}`
                : undefined}
              aria-sort={header.column.getCanSort() ? ariaSort(sorted) : undefined}
            >
              {#if header.column.getCanSort()}
                <button
                  class="dt-th"
                  class:sorted={!!sorted}
                  onclick={header.column.getToggleSortingHandler()}
                >
                  {header.column.columnDef.header}
                  <svg
                    class="dt-caret {sorted || 'none'}"
                    width="8"
                    height="8"
                    viewBox="0 0 10 10"
                    aria-hidden="true"
                  >
                    <path
                      d="M2.5 4 L5 6.5 L7.5 4"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.4"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                </button>
              {:else}
                {header.column.columnDef.header}
              {/if}
            </th>
          {/each}
        </tr>
      {/each}
    </thead>
    <tbody>
      {#each table.getRowModel().rows as r (r.id)}
        {@render row(r.original)}
      {/each}
      {#if table.getRowModel().rows.length === 0 && empty}
        {@render empty()}
      {/if}
    </tbody>
  </table>
</div>

<style>
  /* the sortable header label is a button; reset it to read as the <th> text it
     replaces, with a caret that reveals sort direction */
  .dt-th {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: transparent;
    border: none;
    border-radius: 0;
    box-shadow: none;
    padding: 0;
    margin: 0;
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
    color: inherit;
    cursor: pointer;
  }
  .dt-th:hover {
    color: var(--fg);
    background: transparent;
  }
  /* the glyph is a down chevron; descending keeps it, ascending flips it up,
     unsorted dims it to a neutral hint */
  .dt-caret {
    flex-shrink: 0;
    transition: transform 0.12s ease;
  }
  .dt-caret.none {
    opacity: 0.28;
  }
  .dt-caret.asc {
    transform: rotate(180deg);
  }
  .dt-th.sorted {
    color: var(--fg);
  }
</style>
