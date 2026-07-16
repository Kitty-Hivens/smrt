<script lang="ts">
  import { api, setUnauthorizedHandler } from './lib/api';
  import { t } from './lib/i18n.svelte';
  import { route } from './lib/route.svelte';
  import { isOperator } from './lib/roles';
  import { terms } from './lib/terms.svelte';
  import Login from './views/Login.svelte';
  import AppShell from './views/AppShell.svelte';
  import Browse from './views/Browse.svelte';
  import PublicBrowse from './views/PublicBrowse.svelte';
  import ModPage from './views/ModPage.svelte';
  import Profile from './views/Profile.svelte';
  import MyPacks from './views/MyPacks.svelte';
  import DialogHost from './views/DialogHost.svelte';

  type Me = { uid: number; login: string; role: string; accepted_terms: boolean };
  // undefined = still checking; null = a guest (not signed in); object = identity
  let me = $state<Me | null | undefined>(undefined);
  let showLogin = $state(false);

  $effect(() => {
    api.me().then((m) => {
      me = m;
      if (m) terms.init(m.accepted_terms);
    });
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
    {#if route.mod != null}
      <ModPage modRef={route.mod} me={me ?? null} onBack={() => route.closeMod()} />
    {:else if route.section === 'graph' && me}
      <!-- read-only for a member, full (with debug authoring) for an operator; the
           view gates its own write affordances, so one component serves both.
           lazy: Svelte Flow + dagre are ~200KB, loaded only when the graph opens -->
      {#await import('./views/GraphView.svelte')}
        <div class="muted mono">{t('common.loading')}</div>
      {:then { default: GraphView }}
        <GraphView />
      {/await}
    {:else if route.section === 'profile' && me}
      <Profile {me} />
    {:else if route.section === 'mypacks' && me}
      <MyPacks {me} />
    {:else if me && isOperator(me.role) && route.section !== 'browse'}
      <Browse {me} />
    {:else}
      <PublicBrowse me={me ?? null} onSignIn={() => (showLogin = true)} />
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
