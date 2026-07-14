<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import Avatar from './Avatar.svelte';

  // The signed-in identity and role. Grants were dropped from the role model,
  // so this is the whole of the profile: who you are and what you are.
  type Me = { uid: number; login: string; role: string };
  let { me }: { me: Me } = $props();
</script>

<div class="view">
  <div class="panel card">
    <Avatar uid={me.uid} login={me.login} size={72} />
    <div class="info">
      <div class="login">{me.login}</div>
      <div class="meta muted mono">uid {me.uid}</div>
    </div>
    <span class="chip role-{me.role}">{me.role}</span>
  </div>
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .card {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-5);
  }
  .info {
    flex: 1;
    min-width: 0;
  }
  .login {
    font-size: 18px;
    font-weight: 700;
  }
  .meta {
    font-size: 12px;
    margin-top: 3px;
  }
  .chip {
    font-size: 10px;
    padding: 2px 10px;
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
  .chip.role-debug {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 45%, var(--seam));
    background: var(--warn-soft);
  }
</style>
