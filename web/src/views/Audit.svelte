<script lang="ts">
  import { api } from '../lib/api';
  import { notifyFail } from '../lib/toasts.svelte';
  import { t } from '../lib/i18n.svelte';
  import type { AuditRow } from '../lib/types';

  let rows = $state<AuditRow[]>([]);
  let loading = $state(true);

  async function load() {
    loading = true;
    try {
      rows = await api.auditLog();
    } catch (e) {
      notifyFail(e);
    } finally {
      loading = false;
    }
  }
  load();

  // unix seconds -> "YYYY-MM-DD HH:MM" (UTC), enough to place an action in time
  function when(unix: number): string {
    return new Date(unix * 1000).toISOString().slice(0, 16).replace('T', ' ');
  }
</script>

<div class="view">

  <div class="panel alist">
    {#each rows as r (r.id)}
      <div class="arow">
        <span class="chip mono">{r.action}</span>
        <div class="ainfo">
          <div class="who">
            {r.actor_login} <span class="muted mono">uid {r.actor_uid}</span>
          </div>
          {#if r.target}
            <div class="tgt muted mono">
              {r.target}{#if r.detail}&nbsp;&middot; {r.detail}{/if}
            </div>
          {/if}
        </div>
        <span class="when muted mono">{when(r.created_at)}</span>
      </div>
    {/each}
    {#if rows.length === 0 && !loading}
      <div class="empty muted">{t('audit.empty')}</div>
    {/if}
  </div>
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .alist {
    overflow: hidden;
  }
  .arow {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    border-bottom: 1px solid var(--seam);
  }
  .arow:last-child {
    border-bottom: none;
  }
  .chip {
    font-size: var(--fs-xs);
    padding: 1px 8px;
    border: 1px solid var(--seam);
    border-radius: 999px;
    color: var(--fg-dim);
    flex-shrink: 0;
    letter-spacing: 0.04em;
    align-self: flex-start;
    margin-top: 2px;
  }
  .ainfo {
    flex: 1;
    min-width: 0;
  }
  .who {
    font-size: var(--fs-md);
    font-weight: 600;
  }
  .tgt {
    font-size: var(--fs-xs);
    margin-top: 2px;
    overflow-wrap: anywhere;
  }
  .when {
    font-size: var(--fs-xs);
    flex-shrink: 0;
    align-self: flex-start;
    margin-top: 3px;
  }
  .empty {
    padding: var(--space-4);
    font-size: var(--fs-sm);
  }
</style>
