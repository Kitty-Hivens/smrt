<script lang="ts">
  import type { Snippet } from 'svelte';
  import { api } from '../lib/api';
  import type { Health } from '../lib/types';
  import { route, visibleSections, type Section } from '../lib/route.svelte';
  import { reload } from '../lib/reload.svelte';
  import { activity } from '../lib/motion.svelte';
  import { t, i18n, LOCALES } from '../lib/i18n.svelte';
  import Avatar from './Avatar.svelte';

  type Me = { uid: number; login: string; role: string };
  let {
    me,
    onSignIn,
    onLogout,
    children,
  }: {
    me: Me | null;
    onSignIn: () => void;
    onLogout: () => void;
    children: Snippet;
  } = $props();


  // Off-canvas rail on phones: the topbar burger toggles it; selecting a
  // section, pressing Esc, or tapping the scrim closes it.
  let drawerOpen = $state(false);
  function closeDrawer() {
    drawerOpen = false;
  }
  function onWindowKeydown(e: KeyboardEvent) {
    if (drawerOpen && e.key === 'Escape') drawerOpen = false;
  }

  let health = $state<Health | null>(null);

  $effect(() => {
    api
      .health()
      .then((h) => (health = h))
      .catch(() => {
        // health is footer garnish; a failure here should not block the shell
      });
  });

  // Keep the active section within what this role may see: a stored operator
  // section from a prior admin session shouldn't strand a guest on a blank tab.
  $effect(() => {
    if (!visibleSections(me).includes(route.section)) route.go('browse', true);
  });

  const navKey: Record<Section, Parameters<typeof t>[0]> = {
    browse: 'nav.browse',
    overview: 'nav.overview',
    packs: 'nav.packs',
    servers: 'nav.servers',
    mods: 'nav.mods',
    graph: 'nav.graph',
    users: 'nav.users',
    moderation: 'nav.moderation',
    audit: 'nav.audit',
    profile: 'nav.profile',
    mypacks: 'nav.mypacks',
  };
</script>

<svelte:window onkeydown={onWindowKeydown} />

<div class="shell">
  <nav class="rail" class:open={drawerOpen} id="rail-nav">
    <div class="brand"><span class="mk"></span>smrt<span class="faint">/control</span></div>

    <ul class="nav">
      {#each visibleSections(me) as s}
        <li>
          <button
            class="item"
            class:active={route.section === s}
            aria-current={route.section === s ? 'page' : undefined}
            onclick={() => {
              route.go(s);
              closeDrawer();
            }}
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
      {#if me}
        <button class="who" onclick={() => route.go('profile')} title={t('nav.profile')}>
          <Avatar uid={me.uid} login={me.login} size={26} />
          <span class="whotext faint mono">{me.login} &middot; {me.role}</span>
        </button>
        <button class="signout" onclick={onLogout}>{t('shell.signOut')}</button>
      {:else}
        <button class="signin primary" onclick={onSignIn}>{t('shell.signIn')}</button>
      {/if}
    </div>
  </nav>

  <div class="scrim" class:show={drawerOpen} onclick={closeDrawer} role="presentation"></div>

  <div class="main">
    <header class="topbar">
      <!-- work in flight, shown once for the whole app rather than as a spinner
           per view: the mirror spends most of its time waiting on somewhere else -->
      <div class="wire" class:on={activity.busy} aria-hidden="true"><span></span></div>
      <button
        class="burger"
        aria-label={t('shell.menu')}
        aria-expanded={drawerOpen}
        aria-controls="rail-nav"
        onclick={() => (drawerOpen = !drawerOpen)}
      >
        <span class="bl" aria-hidden="true"></span>
      </button>
      <div class="crumb">
        <span class="faint">smrt /</span>
        {#if route.mod}
          <button class="crumblink" onclick={() => route.closeMod()}>{t(navKey[route.section])}</button>
          <span class="faint">/</span>
          {t('shell.modPage')}
        {:else}
          {t(navKey[route.section])}
        {/if}
      </div>
      <div class="spacer"></div>
      <div class="tools">
        {#if me}
          <button
            class="refresh"
            class:busy={reload.busy}
            onclick={() => reload.request()}
            disabled={reload.busy}
          >
            <span class="rlabel">{t('shell.refresh')}</span>
            <span class="spin" aria-hidden="true"></span>
          </button>
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
    font-size: var(--fs-lg);
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
    font-size: var(--fs-sm);
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
    background: var(--fg);
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
    font-size: var(--fs-xs);
    padding: 0 var(--space-2);
  }
  .signout {
    width: 100%;
    font-size: var(--fs-sm);
  }
  .signin {
    width: 100%;
    font-size: var(--fs-sm);
  }
  .who {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-width: 0;
    padding: 4px var(--space-2);
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    text-align: left;
  }
  .who:hover {
    background: var(--panel-2);
  }
  .whotext {
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
  }
  .wire {
    position: absolute;
    left: 0;
    right: 0;
    bottom: -1px;
    height: 1px;
    overflow: hidden;
    opacity: 0;
    transition: opacity var(--dur-enter) var(--ease-out);
  }
  .wire.on {
    opacity: 1;
  }
  .wire span {
    display: block;
    width: 25%;
    height: 100%;
    background: var(--fg);
    animation: wire-sweep 1.1s linear infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    /* no travelling hairline: the wire sits lit while work is in flight */
    .wire span {
      width: 100%;
      animation: none;
      opacity: 0.5;
    }
  }
  .topbar {
    position: relative;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    row-gap: var(--space-2);
    padding: var(--space-3) var(--space-5);
    border-bottom: 1px solid var(--seam);
  }
  /* hamburger: hidden until the drawer breakpoint (see @media below) */
  .burger {
    display: none;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 30px;
    padding: 0;
    margin-right: var(--space-3);
  }
  .burger .bl,
  .burger .bl::before,
  .burger .bl::after {
    width: 16px;
    height: 1.5px;
    background: var(--fg);
    border-radius: 2px;
  }
  .burger .bl {
    position: relative;
    display: block;
  }
  .burger .bl::before,
  .burger .bl::after {
    content: '';
    position: absolute;
    left: 0;
  }
  .burger .bl::before {
    top: -5px;
  }
  .burger .bl::after {
    top: 5px;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  /* a crumb segment you can walk back to reads as a control, not as text */
  .crumblink {
    border: none;
    background: transparent;
    padding: 0;
    min-height: 0;
    font: inherit;
    color: var(--fg-dim);
    cursor: pointer;
  }
  .crumblink:hover {
    color: var(--fg);
    background: transparent;
  }
  .crumb {
    font-family: var(--mono);
    font-size: var(--fs-sm);
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
    font-size: var(--fs-xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 6px 12px;
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
    font-size: var(--fs-xs);
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

  /* scrim base -- inert until the drawer breakpoint */
  .scrim {
    display: none;
  }

  /* strip mode -- tablet / narrow laptop (561-768px): the rail becomes a
     horizontal top bar. Brand pinned left, foot pinned right, nav scrolls
     between them. Nothing is dropped. */
  @media (min-width: 561px) and (max-width: 768px) {
    .shell {
      /* minmax(0,...) not 1fr: 1fr's auto minimum lets the horizontal rail's
         min-content (the full nav) blow the single column past the viewport */
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: auto minmax(0, 1fr);
    }
    .rail {
      flex-direction: row;
      align-items: center;
      gap: var(--space-3);
      padding: var(--space-2) var(--space-3);
      border-right: none;
      border-bottom: 1px solid var(--seam);
    }
    .brand {
      flex: none;
      padding: 0 var(--space-2);
    }
    .nav {
      flex-direction: row;
      flex: 1;
      min-width: 0;
      overflow-x: auto;
      gap: var(--space-1);
    }
    .nav li {
      flex: none;
    }
    .item {
      width: auto;
      white-space: nowrap;
    }
    .item.active::before {
      top: auto;
      bottom: 2px;
      left: 8px;
      right: 8px;
      width: auto;
      height: 3px;
    }
    .spacer {
      display: none;
    }
    .foot {
      flex-direction: row;
      align-items: center;
      flex: none;
      padding: 0;
      gap: var(--space-2);
    }
    .who,
    .signout,
    .signin {
      width: auto;
    }
    .whotext {
      max-width: 140px;
    }
  }

  /* drawer mode -- phone (<=560px): the rail slides in from the left over a
     scrim, toggled by the topbar burger. The full vertical rail is preserved. */
  @media (max-width: 560px) {
    .shell {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(0, 1fr);
    }
    .rail {
      position: fixed;
      top: 0;
      left: 0;
      bottom: 0;
      z-index: 50;
      width: min(280px, 82vw);
      background: var(--panel);
      overflow-y: auto;
      transform: translateX(-100%);
      transition: transform var(--dur-state) var(--ease-out);
    }
    .rail.open {
      transform: translateX(0);
      box-shadow: var(--shadow-pop);
    }
    .burger {
      display: inline-flex;
    }
    .scrim {
      display: block;
      position: fixed;
      inset: 0;
      z-index: 40;
      background: rgba(0, 0, 0, 0.6);
      opacity: 0;
      visibility: hidden;
      transition:
        opacity var(--dur-state) var(--ease-out),
        visibility 0.2s ease;
    }
    .scrim.show {
      opacity: 1;
      visibility: visible;
    }
    .topbar {
      padding: var(--space-3) var(--space-4);
    }
    .tools {
      flex-basis: 100%;
    }
    .content {
      padding: var(--space-3);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .rail,
    .scrim {
      transition: none;
    }
  }
</style>
