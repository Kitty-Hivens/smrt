<script lang="ts">
  import { api } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import type { UserRow } from '../lib/types';
  import Avatar from './Avatar.svelte';

  let users = $state<UserRow[]>([]);
  let meUid = $state<number | null>(null);
  let err = $state('');
  let loading = $state(true);

  async function load() {
    loading = true;
    err = '';
    try {
      const [u, me] = await Promise.all([api.listUsers(), api.me()]);
      users = u;
      meUid = me?.uid ?? null;
    } catch (e) {
      err = String(e);
    } finally {
      loading = false;
    }
  }
  load();

  async function setRole(u: UserRow, role: string) {
    err = '';
    try {
      await api.setUserRole(u.github_uid, role);
      await load();
    } catch (e) {
      err = String(e);
    }
  }

  // last-login timestamps are unix seconds; 0 marks a row that never logged in
  function seen(unix: number): string {
    if (!unix) return t('users.never');
    return new Date(unix * 1000).toISOString().slice(0, 10);
  }
</script>

<div class="view">
  {#if err}<div class="err mono">{err}</div>{/if}

  <div class="panel ulist">
    {#each users as u (u.github_uid)}
      <div class="urow">
        <Avatar uid={u.github_uid} login={u.login} size={32} />
        <div class="uinfo">
          <div class="uname">
            {u.login}{#if u.github_uid === meUid}<span class="me mono">{t('users.you')}</span>{/if}
          </div>
          <div class="umeta muted mono">
            uid {u.github_uid} &middot; {t('users.lastLogin')} {seen(u.last_login_at)}
          </div>
        </div>
        <span class="chip role-{u.role}">{u.role}</span>
        {#if u.github_uid !== meUid}
          {#if u.role === 'admin'}
            <button class="link" onclick={() => setRole(u, 'member')}>{t('users.demote')}</button>
          {:else}
            <button class="link" onclick={() => setRole(u, 'admin')}>{t('users.promote')}</button>
          {/if}
        {/if}
      </div>
    {/each}
    {#if users.length === 0 && !loading}
      <div class="empty muted">{t('users.empty')}</div>
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
  .ulist {
    overflow: hidden;
  }
  .urow {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    border-bottom: 1px solid var(--seam);
  }
  .urow:last-child {
    border-bottom: none;
  }
  .uinfo {
    flex: 1;
    min-width: 0;
  }
  .uname {
    font-size: 14px;
    font-weight: 600;
  }
  .me {
    margin-left: 6px;
    font-size: 10px;
    color: var(--fg-faint);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-weight: 400;
  }
  .umeta {
    font-size: 11px;
    margin-top: 2px;
  }
  .chip {
    font-size: 10px;
    padding: 1px 8px;
    border: 1px solid var(--seam);
    border-radius: 999px;
    color: var(--fg-dim);
    flex-shrink: 0;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-family: var(--mono);
  }
  .chip.role-admin {
    color: var(--info);
    border-color: color-mix(in srgb, var(--info) 45%, var(--seam));
    background: var(--info-soft);
  }
  .link {
    background: transparent;
    border: none;
    border-radius: 0;
    color: var(--fg-dim);
    padding: 4px 8px;
    font-size: 11px;
    flex-shrink: 0;
  }
  .link:hover {
    color: var(--fg);
  }
  .empty {
    padding: var(--space-4);
    font-size: 12px;
  }
</style>
