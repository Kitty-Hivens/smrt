<script lang="ts">
  import { api, setUnauthorizedHandler } from './lib/api';
  import { t } from './lib/i18n.svelte';
  import { route } from './lib/route.svelte';
  import Login from './views/Login.svelte';
  import AppShell from './views/AppShell.svelte';
  import Browse from './views/Browse.svelte';
  import PublicBrowse from './views/PublicBrowse.svelte';
  import DialogHost from './views/DialogHost.svelte';

  type Me = { uid: number; login: string; role: string };
  // undefined = still checking; null = a guest (not signed in); object = identity
  let me = $state<Me | null | undefined>(undefined);
  let showLogin = $state(false);

  $effect(() => {
    api.me().then((m) => (me = m));
  });

  // A 401 on an authed call (expired session) drops back to the guest view.
  setUnauthorizedHandler(() => {
    me = null;
  });

  async function logout() {
    await api.logout();
    me = null;
  }
</script>

{#if me === undefined}
  <div class="boot"><span class="muted mono">{t('app.checkingSession')}</span></div>
{:else if showLogin}
  <Login onClose={() => (showLogin = false)} />
{:else}
  <AppShell me={me ?? null} onSignIn={() => (showLogin = true)} onLogout={logout}>
    {#if me?.role === 'admin' && route.section !== 'browse'}
      <Browse />
    {:else}
      <PublicBrowse />
    {/if}
  </AppShell>
{/if}

<DialogHost />

<style>
  .boot {
    display: grid;
    place-items: center;
    height: 100%;
  }
</style>
