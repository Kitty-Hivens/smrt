<script lang="ts">
  import { api, setUnauthorizedHandler } from './lib/api';
  import { t } from './lib/i18n.svelte';
  import Login from './views/Login.svelte';
  import AppShell from './views/AppShell.svelte';
  import Browse from './views/Browse.svelte';
  import DialogHost from './views/DialogHost.svelte';

  // null = still checking the session
  let authed = $state<boolean | null>(null);

  $effect(() => {
    api.session().then((ok) => (authed = ok));
  });

  // A 401 from any authed call (expired cookie) bounces back to login.
  setUnauthorizedHandler(() => {
    authed = false;
  });

  async function logout() {
    await api.logout();
    authed = false;
  }
</script>

{#if authed === null}
  <div class="boot"><span class="muted mono">{t('app.checkingSession')}</span></div>
{:else if !authed}
  <Login onAuthed={() => (authed = true)} />
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
</style>
