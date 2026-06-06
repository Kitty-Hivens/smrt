<script lang="ts">
  import { api } from '../lib/api';
  import { t, i18n, LOCALES } from '../lib/i18n.svelte';

  let { onAuthed }: { onAuthed: () => void } = $props();
  let token = $state('');
  let busy = $state(false);
  let error = $state('');

  async function submit(e: Event) {
    e.preventDefault();
    if (!token.trim()) return;
    busy = true;
    error = '';
    const ok = await api.login(token.trim());
    busy = false;
    if (ok) onAuthed();
    else error = t('login.rejected');
  }
</script>

<div class="wrap">
  <div class="locale" role="group" aria-label={t('shell.locale')}>
    {#each LOCALES as loc}
      <button
        class="loc"
        class:active={i18n.locale === loc}
        aria-pressed={i18n.locale === loc}
        onclick={() => i18n.set(loc)}
      >
        {loc.toUpperCase()}
      </button>
    {/each}
  </div>

  <form class="panel card" onsubmit={submit}>
    <div class="brand mono">smrt<span class="faint">/control</span></div>
    <p class="muted sub">{t('login.subtitle')}</p>
    <input type="password" bind:value={token} placeholder="SMRT_ADMIN_TOKEN" autocomplete="off" />
    {#if error}<div class="err mono">{error}</div>{/if}
    <button class="primary" type="submit" disabled={busy || !token.trim()}>
      {busy ? t('login.checking') : t('login.submit')}
    </button>
  </form>
  <div class="foot faint mono">{t('login.foot')}</div>
</div>

<style>
  .wrap {
    position: relative;
    display: grid;
    place-items: center;
    align-content: center;
    height: 100%;
    gap: var(--space-5);
  }
  .locale {
    position: absolute;
    top: var(--space-5);
    right: var(--space-5);
    display: inline-flex;
    border: 1px solid var(--seam-bright);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }
  .loc {
    border: none;
    border-radius: 0;
    padding: 5px 11px;
    font-size: 11.5px;
    letter-spacing: 0.04em;
    color: var(--fg-dim);
    background: transparent;
  }
  .loc:hover {
    background: var(--panel-2);
  }
  .loc.active {
    background: var(--accent-soft);
    color: var(--accent-strong);
  }
  .card {
    width: 360px;
    max-width: 92vw;
    padding: var(--space-6);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .brand {
    font-size: 22px;
    letter-spacing: 0.04em;
  }
  .sub {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
  }
  .err {
    color: var(--danger);
    font-size: 12px;
  }
  .foot {
    font-size: 11px;
  }
</style>
