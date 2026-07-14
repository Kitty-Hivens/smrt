<script lang="ts">
  import { api } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import type { AuditRow } from '../lib/types';

  let rows = $state<AuditRow[]>([]);
  let err = $state('');
  let loading = $state(true);

  async function load() {
    loading = true;
    err = '';
    try {
      rows = await api.auditLog();
    } catch (e) {
      err = String(e);
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
  {#if err}<div class="err mono">{err}</div>{/if}

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
  .err {
    color: var(--danger);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: var(--radius-sm);
    padding: var(--space-3) var(--space-4);
    font-size: 12px;
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
    font-size: 10px;
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
    font-size: 13px;
    font-weight: 600;
  }
  .tgt {
    font-size: 11px;
    margin-top: 2px;
    overflow-wrap: anywhere;
  }
  .when {
    font-size: 11px;
    flex-shrink: 0;
    align-self: flex-start;
    margin-top: 3px;
  }
  .empty {
    padding: var(--space-4);
    font-size: 12px;
  }
</style>
