<script lang="ts">
  import { api } from '../lib/api';
  import { t, i18n, LOCALES } from '../lib/i18n.svelte';

  let { onClose }: { onClose?: () => void } = $props();
  let token = $state('');
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let showToken = $state(false);

  // Surface an OAuth outcome the callback handed back, then wipe it from the URL
  // so a reload doesn't replay the message.
  const authParam = new URLSearchParams(window.location.search).get('auth');
  const authKey =
    authParam === 'denied'
      ? 'login.denied'
      : authParam === 'failed'
        ? 'login.failed'
        : authParam === 'unconfigured'
          ? 'login.unconfigured'
          : '';
  if (authParam) history.replaceState({}, '', '/');

  function github() {
    window.location.href = '/v1/auth/github/login';
  }

  // The token form no longer signs anyone in. A valid token is answered with a
  // deprecation notice pointing at GitHub; an invalid one is rejected as before.
  async function submit(e: Event) {
    e.preventDefault();
    if (!token.trim()) return;
    busy = true;
    error = '';
    notice = '';
    const res = await api.login(token.trim());
    busy = false;
    if (res === 'deprecated') notice = t('login.deprecated');
    else error = t('login.rejected');
  }
</script>

<div class="wrap">
  {#if onClose}
    <button class="back mono" onclick={onClose}>&larr; {t('login.back')}</button>
  {/if}
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

  <div class="panel card">
    <div class="brand"><span class="mk"></span>smrt<span class="faint">/control</span></div>
    <p class="muted sub">{t('login.subtitle')}</p>

    {#if authKey}<div class="err mono">{t(authKey)}</div>{/if}

    <button class="primary gh" onclick={github}>
      <svg class="ghmark" viewBox="0 0 16 16" aria-hidden="true">
        <path
          fill="currentColor"
          d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82a7.6 7.6 0 0 1 2-.27c.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8z"
        />
      </svg>
      {t('login.github')}
    </button>

    {#if !showToken}
      <button class="toggle" onclick={() => (showToken = true)}>{t('login.useToken')}</button>
    {:else}
      <div class="divider"><span>{t('login.or')}</span></div>
      <form class="tokenform" onsubmit={submit}>
        <input
          type="password"
          bind:value={token}
          placeholder="SMRT_ADMIN_TOKEN" aria-label="SMRT_ADMIN_TOKEN"
          autocomplete="off"
        />
        {#if error}<div class="err mono">{error}</div>{/if}
        {#if notice}<div class="note mono">{notice}</div>{/if}
        <button class="primary" type="submit" disabled={busy || !token.trim()}>
          {busy ? t('login.checking') : t('login.submit')}
        </button>
      </form>
    {/if}
  </div>
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
    background-color: var(--bg);
    background-image: radial-gradient(var(--dotfield) 1px, transparent 1px);
    background-size: 20px 20px;
  }
  .back {
    position: absolute;
    top: var(--space-5);
    left: var(--space-5);
    background: transparent;
    border: none;
    border-radius: 0;
    color: var(--fg-faint);
    font-size: var(--fs-sm);
    padding: 4px 6px;
  }
  .back:hover {
    color: var(--fg-dim);
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
    font-size: var(--fs-xs);
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
    display: flex;
    align-items: center;
    gap: 10px;
    font-family: var(--mono);
    font-weight: 700;
    font-size: var(--fs-xl);
    letter-spacing: 0;
  }
  .brand .mk {
    width: 24px;
    height: 24px;
    border-radius: 7px;
    background: var(--fg);
  }
  .brand .faint {
    font-weight: 500;
  }
  .sub {
    margin: 0;
    font-size: var(--fs-md);
    line-height: 1.5;
  }
  .gh {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 9px;
  }
  .ghmark {
    width: 16px;
    height: 16px;
    flex: none;
  }
  .toggle {
    align-self: center;
    background: transparent;
    border: none;
    border-radius: 0;
    color: var(--fg-faint);
    font-size: var(--fs-xs);
    padding: 2px 6px;
  }
  .toggle:hover {
    color: var(--fg-dim);
  }
  .divider {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    color: var(--fg-faint);
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .divider::before,
  .divider::after {
    content: '';
    flex: 1;
    height: 1px;
    background: var(--seam);
  }
  .tokenform {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .note {
    color: var(--fg-dim);
    font-size: var(--fs-sm);
    line-height: 1.5;
  }
  .foot {
    font-size: var(--fs-xs);
  }
</style>
