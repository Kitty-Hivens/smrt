<script lang="ts">
  import { api } from './lib/api';
  import Login from './views/Login.svelte';
  import Browse from './views/Browse.svelte';

  // null = still checking the session
  let authed = $state<boolean | null>(null);

  $effect(() => {
    api.session().then((ok) => (authed = ok));
  });

  async function logout() {
    await api.logout();
    authed = false;
  }
</script>

{#if authed === null}
  <div class="boot"><span class="muted mono">checking session...</span></div>
{:else if !authed}
  <Login onAuthed={() => (authed = true)} />
{:else}
  <Browse onLogout={logout} />
{/if}

<style>
  .boot {
    display: grid;
    place-items: center;
    height: 100%;
  }
</style>
