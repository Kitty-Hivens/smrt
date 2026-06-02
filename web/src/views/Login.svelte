<script lang="ts">
  import { api } from '../lib/api';

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
    else error = 'Rejected. Check the admin token.';
  }
</script>

<div class="wrap">
  <form class="panel card" onsubmit={submit}>
    <div class="brand mono">smrt<span class="faint">/control</span></div>
    <p class="muted sub">Mirror admin. Paste the admin token to continue.</p>
    <input
      type="password"
      bind:value={token}
      placeholder="SMRT_ADMIN_TOKEN"
      autocomplete="off"
    />
    {#if error}<div class="err mono">{error}</div>{/if}
    <button class="primary" type="submit" disabled={busy || !token.trim()}>
      {busy ? 'checking...' : 'Enter'}
    </button>
  </form>
  <div class="foot faint mono">smrt mirror control panel</div>
</div>

<style>
  .wrap {
    display: grid;
    place-items: center;
    align-content: center;
    height: 100%;
    gap: 18px;
  }
  .card {
    width: 340px;
    padding: 28px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .brand {
    font-size: 22px;
    letter-spacing: 0.04em;
  }
  .sub {
    margin: 0;
    font-size: 13px;
  }
  .err {
    color: var(--danger);
    font-size: 12px;
  }
  .foot {
    font-size: 11px;
  }
</style>
