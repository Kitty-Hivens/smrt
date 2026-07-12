<script lang="ts">
  // Avatar tile: the GitHub avatar proxied through the mirror (/v1/users/:uid/avatar),
  // falling back to a letter tile when there is no uid or the fetch fails (guest,
  // reserved uid 0, or an upstream miss).
  let { uid, login, size = 32 }: { uid: number; login: string; size?: number } = $props();
  let failed = $state(false);
  const letter = $derived((login?.[0] ?? '?').toUpperCase());
  // reset the error state if the identity changes (e.g. a re-rendered list row)
  $effect(() => {
    void uid;
    failed = false;
  });
</script>

{#if !uid || uid <= 0 || failed}
  <span class="avatar mono" style="width:{size}px;height:{size}px;font-size:{Math.round(size * 0.4)}px"
    >{letter}</span
  >
{:else}
  <img
    class="avatar"
    style="width:{size}px;height:{size}px"
    src="/v1/users/{uid}/avatar"
    alt={login}
    loading="lazy"
    onerror={() => (failed = true)}
  />
{/if}

<style>
  .avatar {
    flex: none;
    border-radius: var(--radius-sm);
    background: var(--panel-3);
    color: var(--fg-dim);
    display: inline-grid;
    place-items: center;
    object-fit: cover;
    font-weight: 700;
    overflow: hidden;
  }
</style>
