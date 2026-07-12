<script lang="ts">
  import { api, setUnauthorizedHandler } from './lib/api';
  import { t } from './lib/i18n.svelte';
  import Login from './views/Login.svelte';
  import AppShell from './views/AppShell.svelte';
  import Browse from './views/Browse.svelte';
  import DialogHost from './views/DialogHost.svelte';

  type Me = { uid: number; login: string; role: string };
  // undefined = still checking; null = not signed in; object = the identity
  let me = $state<Me | null | undefined>(undefined);

  $effect(() => {
    api.me().then((m) => (me = m));
  });

  // A 401 from any authed call (expired session) bounces back to login.
  setUnauthorizedHandler(() => {
    me = null;
  });

  async function refresh() {
    me = await api.me();
  }

  async function logout() {
    await api.logout();
    me = null;
  }
</script>

{#if me === undefined}
  <div class="boot"><span class="muted mono">{t('app.checkingSession')}</span></div>
{:else if me === null}
  <Login onAuthed={refresh} />
{:else if me.role !== 'admin'}
  <div class="boot gate">
    <div class="brand"><span class="mk"></span>smrt<span class="faint">/control</span></div>
    <p class="muted">{t('member.notOperator', { login: me.login })}</p>
    <button class="ghost" onclick={logout}>{t('shell.signOut')}</button>
  </div>
{:else}
  <AppShell onLogout={logout}>
    <Browse />
  </AppShell>
{/if}

<DialogHost />

<style>
  .boot {
    display: grid;
    place-items: center;
    height: 100%;
  }
  .gate {
    align-content: center;
    gap: var(--space-4);
    background-color: var(--bg);
    background-image: radial-gradient(var(--dotfield) 1px, transparent 1px);
    background-size: 20px 20px;
    text-align: center;
  }
  .gate .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    font-family: var(--mono);
    font-weight: 700;
    font-size: 20px;
  }
  .gate .brand .mk {
    width: 24px;
    height: 24px;
    border-radius: 7px;
    background: var(--fg);
  }
  .gate .brand .faint {
    font-weight: 500;
  }
  .gate p {
    margin: 0;
    font-size: 13px;
    max-width: 320px;
  }
</style>
