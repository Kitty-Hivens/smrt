<script lang="ts">
  import type { Snippet } from 'svelte';
  import { api } from '../lib/api';
  import type { Health } from '../lib/types';
  import { route, SECTIONS, type Section } from '../lib/route.svelte';
  import { reload } from '../lib/reload.svelte';
  import { t, i18n, LOCALES } from '../lib/i18n.svelte';

  let { onLogout, children }: { onLogout: () => void; children: Snippet } = $props();

  let health = $state<Health | null>(null);

  $effect(() => {
    api
      .health()
      .then((h) => (health = h))
      .catch(() => {
        // health is footer garnish; a failure here should not block the shell
      });
  });

  const navKey: Record<Section, Parameters<typeof t>[0]> = {
    overview: 'nav.overview',
    packs: 'nav.packs',
    servers: 'nav.servers',
    mods: 'nav.mods',
  };
</script>

<div class="shell">
  <nav class="rail">
    <div class="brand"><span class="mk"></span>smrt<span class="faint">/control</span></div>

    <ul class="nav">
      {#each SECTIONS as s}
        <li>
          <button
            class="item"
            class:active={route.section === s}
            aria-current={route.section === s ? 'page' : undefined}
            onclick={() => route.go(s)}
          >
            {t(navKey[s])}
          </button>
        </li>
      {/each}
    </ul>

    <div class="spacer"></div>

    <div class="foot">
      {#if health}
        <div class="health faint mono">
          {t('shell.health', { version: health.version, schema: health.schema_version })}
        </div>
      {/if}
      <button class="signout" onclick={onLogout}>{t('shell.signOut')}</button>
    </div>
  </nav>

  <div class="main">
    <header class="topbar">
      <div class="crumb"><span class="faint">smrt /</span> {t(navKey[route.section])}</div>
      <div class="spacer"></div>
      <button class="refresh" class:busy={reload.busy} onclick={() => reload.request()} disabled={reload.busy}>
        <span class="rlabel">{t('shell.refresh')}</span>
        <span class="spin" aria-hidden="true"></span>
      </button>
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
    </header>

    <main class="content scroll">
      {@render children()}
    </main>
  </div>
</div>

<style>
  .shell {
    display: grid;
    grid-template-columns: 224px minmax(0, 1fr);
    height: 100%;
    background-color: var(--bg);
    background-image: radial-gradient(var(--dotfield) 1px, transparent 1px);
    background-size: 20px 20px;
  }
  .rail {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-4) var(--space-3);
    border-right: 1px solid var(--seam);
    background: color-mix(in srgb, var(--panel) 55%, transparent);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    font-family: var(--mono);
    font-weight: 700;
    font-size: 15px;
    letter-spacing: 0;
    padding: var(--space-2) var(--space-3) var(--space-5);
  }
  .brand .mk {
    width: 22px;
    height: 22px;
    border-radius: 7px;
    background: var(--fg);
    box-shadow: var(--shadow-1);
    flex: none;
  }
  .brand .faint {
    font-weight: 500;
  }
  .nav {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .item {
    position: relative;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    padding: 10px 12px 10px 14px;
    color: var(--fg-dim);
    font-family: var(--mono);
    font-size: 12px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    box-shadow: none;
  }
  .item:hover {
    background: var(--panel-2);
    color: var(--fg);
  }
  .item.active {
    background: transparent;
    color: var(--fg);
  }
  .item.active::before {
    content: '';
    position: absolute;
    left: 0;
    top: 8px;
    bottom: 8px;
    width: 3px;
    border-radius: 3px;
    background: var(--red);
  }
  .spacer {
    flex: 1;
  }
  .foot {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-2) var(--space-1);
  }
  .health {
    font-size: 11px;
    padding: 0 var(--space-2);
  }
  .signout {
    width: 100%;
    font-size: 12.5px;
  }

  .main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
  }
  .topbar {
    display: flex;
    align-items: center;
    padding: var(--space-3) var(--space-5);
    border-bottom: 1px solid var(--seam);
  }
  .crumb {
    font-family: var(--mono);
    font-size: 12px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--fg);
  }
  .crumb .faint {
    color: var(--fg-faint);
  }
  .refresh {
    position: relative;
    font-family: var(--mono);
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 6px 12px;
    margin-right: var(--space-3);
  }
  .refresh .spin {
    display: none;
  }
  .refresh.busy .rlabel {
    visibility: hidden;
  }
  .refresh.busy .spin {
    display: block;
    position: absolute;
    top: 50%;
    left: 50%;
    margin: -7px 0 0 -7px;
    width: 14px;
    height: 14px;
    border: 2px solid var(--seam-bright);
    border-top-color: var(--fg);
    border-radius: 50%;
    animation: refresh-spin 0.6s linear infinite;
  }
  @keyframes refresh-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .refresh.busy .spin {
      animation: none;
    }
  }
  .locale {
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
    box-shadow: none;
  }
  .loc:hover {
    background: var(--panel-2);
  }
  .loc.active {
    background: var(--accent-soft);
    color: var(--accent-strong);
  }
  .content {
    flex: 1;
    overflow: auto;
    padding: var(--space-5);
  }
</style>
