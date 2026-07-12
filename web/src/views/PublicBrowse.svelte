<script lang="ts">
  import { api } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { renderMarkdown } from '../lib/markdown';
  import ModIcon from './ModIcon.svelte';
  import type { PackManifest, PackSummary } from '../lib/types';

  // Guest-facing, read-only. Everything here is the public launcher contract:
  // /v1/packs and /v1/packs/:id/manifest -- no gated calls, no authoring.
  let packs = $state<PackSummary[]>([]);
  let loading = $state(true);
  let err = $state('');

  let openId = $state<string | null>(null);
  let manifest = $state<PackManifest | null>(null);
  let mLoading = $state(false);

  async function load() {
    loading = true;
    err = '';
    try {
      packs = (await api.packs()).packs;
    } catch (e) {
      err = String(e);
    } finally {
      loading = false;
    }
  }
  load();

  async function open(p: PackSummary) {
    if (openId === p.pack_id) {
      openId = null;
      manifest = null;
      return;
    }
    openId = p.pack_id;
    manifest = null;
    mLoading = true;
    try {
      manifest = await api.manifest(p.pack_id);
    } catch (e) {
      err = String(e);
    } finally {
      mLoading = false;
    }
  }

  function modName(m: PackManifest['mods'][number]): string {
    return m.display?.name ?? m.filename.replace(/\.jar$/, '');
  }
</script>

<div class="view">
  {#if err}<div class="err mono">{err}</div>{/if}

  <div class="panel plist">
    {#each packs as p (p.pack_id)}
      <div class="pack" class:open={openId === p.pack_id}>
        <button class="prow" onclick={() => open(p)}>
          <span class="chev" aria-hidden="true">&#9656;</span>
          {#if p.icon_url}
            <img class="picon" src={p.icon_url} alt={p.display_name} loading="lazy" />
          {:else}
            <span class="picon avatar mono">{p.display_name.slice(0, 1).toUpperCase()}</span>
          {/if}
          <div class="pinfo">
            <div class="pname">
              {p.display_name}{#if p.featured}<span class="feat mono">{t('packs.flag.featured')}</span>{/if}
            </div>
            {#if p.tagline}<div class="ptag muted">{p.tagline}</div>{/if}
          </div>
          <div class="pmeta">
            <span class="tag">{p.minecraft_version}</span>
            <span class="pver mono faint">{p.latest_pack_version}</span>
          </div>
        </button>

        {#if openId === p.pack_id}
          <div class="detail">
            {#if mLoading}
              <div class="muted s">{t('common.loading')}</div>
            {:else if manifest}
              {#if p.description_md}
                <!-- renderMarkdown sanitizes; safe to inject -->
                <div class="desc">{@html renderMarkdown(p.description_md)}</div>
              {/if}
              <div class="modhead mono faint">{t('browse.modsN', { n: manifest.mods.length })}</div>
              <div class="mods">
                {#each manifest.mods as m (m.sha1)}
                  <div class="mrow">
                    <ModIcon
                      name={modName(m)}
                      source={m.source}
                      iconUrl={m.display?.icon_url ?? null}
                      size={24}
                      mono
                    />
                    <span class="mn">{modName(m)}</span>
                    {#if !m.required}<span class="opt mono">{t('browse.optional')}</span>{/if}
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
      </div>
    {/each}
    {#if packs.length === 0 && !loading}
      <div class="empty muted">{t('browse.empty')}</div>
    {/if}
  </div>
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .err {
    color: var(--danger);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: var(--radius-sm);
    padding: var(--space-3) var(--space-4);
    font-size: 12px;
  }
  .plist {
    overflow: hidden;
  }
  .pack {
    border-bottom: 1px solid var(--seam);
  }
  .pack:last-child {
    border-bottom: none;
  }
  .prow {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-radius: 0;
    padding: var(--space-3);
    cursor: pointer;
  }
  .prow:hover {
    background: var(--panel-2);
  }
  .chev {
    color: var(--fg-faint);
    font-size: 11px;
    flex: none;
    transition: transform 0.15s ease;
  }
  .pack.open .chev {
    transform: rotate(90deg);
    color: var(--fg-dim);
  }
  .picon {
    width: 34px;
    height: 34px;
    flex: none;
    border-radius: var(--radius-sm);
    object-fit: cover;
  }
  .avatar {
    display: grid;
    place-items: center;
    background: var(--panel-3);
    color: var(--fg-dim);
    font-size: 14px;
  }
  .pinfo {
    flex: 1;
    min-width: 0;
  }
  .pname {
    font-size: 14px;
    font-weight: 600;
  }
  .feat {
    margin-left: 8px;
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--accent);
    font-weight: 400;
  }
  .ptag {
    font-size: 12px;
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pmeta {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-shrink: 0;
  }
  .pver {
    font-size: 11px;
  }
  .detail {
    padding: 2px var(--space-3) var(--space-4) 42px;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .desc {
    font-size: 13px;
    line-height: 1.55;
    color: var(--fg-dim);
    max-width: 65ch;
  }
  .modhead {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .mods {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 6px var(--space-4);
  }
  .mrow {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .mn {
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .opt {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-faint);
    flex-shrink: 0;
  }
  .empty,
  .s {
    padding: var(--space-4);
    font-size: 12px;
  }
</style>
