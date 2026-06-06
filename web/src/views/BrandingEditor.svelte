<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { dialogs } from '../lib/dialogs.svelte';
  import { t } from '../lib/i18n.svelte';
  import DropZone from './ui/DropZone.svelte';

  let { packId }: { packId: string } = $props();

  // where a dropped file lands: the two branding images get stable names so the
  // curator URL stays put; everything else keeps its own filename
  type Dest = 'icon' | 'banner' | 'asset';
  let dest = $state<Dest>('asset');
  let files = $state<string[]>([]);
  let busy = $state(false);
  let err = $state('');

  async function load() {
    err = '';
    try {
      files = (await api.packStatic(packId)).files;
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 404)) {
        err = e instanceof ApiError ? `${e.status} ${e.body}` : String(e);
      }
      files = [];
    }
  }
  load();

  const ext = (name: string) => name.split('.').pop()?.toLowerCase() || 'png';

  function destFor(file: File): string {
    if (dest === 'icon') return `_nexira/icon.${ext(file.name)}`;
    if (dest === 'banner') return `_nexira/banner.${ext(file.name)}`;
    return `_nexira/${file.name}`;
  }

  async function onDrop(dropped: File[]) {
    // icon / banner are single-target; an asset drop may carry many
    const list = dest === 'asset' ? dropped : dropped.slice(0, 1);
    busy = true;
    err = '';
    try {
      for (const file of list) {
        await api.uploadStatic(packId, destFor(file), file);
      }
      await load();
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    } finally {
      busy = false;
    }
  }

  async function del(f: string) {
    const ok = await dialogs.confirm(t('be.deleteMsg', { file: f }), {
      title: t('be.deleteTitle'),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteStatic(packId, f);
      await load();
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    }
  }

  const isImage = (f: string) => /\.(png|jpe?g|gif|webp|svg)$/i.test(f);

  const modes: Dest[] = ['icon', 'banner', 'asset'];
  const modeKey: Record<Dest, Parameters<typeof t>[0]> = {
    icon: 'be.icon',
    banner: 'be.banner',
    asset: 'be.asset',
  };
  const dropLabel = $derived(
    busy
      ? t('pe.uploading')
      : dest === 'icon'
        ? t('be.dropIcon')
        : dest === 'banner'
          ? t('be.dropBanner')
          : t('be.dropAsset'),
  );
</script>

<p class="muted hint">{t('be.hint')}</p>

<div class="modes" role="group" aria-label={t('be.dropAs')}>
  <span class="ml">{t('be.dropAs')}</span>
  {#each modes as m}
    <button class="mode" class:active={dest === m} aria-pressed={dest === m} onclick={() => (dest = m)}>
      {t(modeKey[m])}
    </button>
  {/each}
</div>

<DropZone
  label={dropLabel}
  accept={dest === 'asset' ? undefined : 'image/*'}
  multiple={dest === 'asset'}
  {busy}
  onFiles={onDrop}
/>
<div class="formats faint">{t('be.formats')}</div>

{#if err}<div class="err mono">{err}</div>{/if}

<div class="grid">
  {#each files as f}
    <div class="card panel">
      {#if isImage(f)}
        <img src={api.staticUrl(packId, f)} alt={f} />
      {:else}
        <div class="ext mono">.{f.split('.').pop()}</div>
      {/if}
      <div class="meta">
        <div class="fn mono" title={f}>{f}</div>
        <div class="row2">
          <a class="mono" href={api.staticUrl(packId, f)} target="_blank" rel="noreferrer">{t('be.open')}</a>
          <button class="danger sm" onclick={() => del(f)}>{t('common.delete')}</button>
        </div>
      </div>
    </div>
  {/each}
  {#if files.length === 0}<div class="muted">{t('be.empty')}</div>{/if}
</div>

<style>
  .hint {
    font-size: 12px;
    margin: 0 0 var(--space-3);
    max-width: 720px;
    line-height: 1.5;
  }
  .modes {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }
  .ml {
    font-size: 12px;
    color: var(--fg-dim);
    margin-right: var(--space-1);
  }
  .mode {
    padding: 5px 12px;
    font-size: 12.5px;
    color: var(--fg-dim);
    background: transparent;
  }
  .mode.active {
    background: var(--accent-soft);
    color: var(--accent-strong);
    border-color: var(--accent-dim);
  }
  .formats {
    font-size: 11px;
    margin: var(--space-2) 0 var(--space-4);
  }
  .err {
    color: var(--danger);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    font-size: 12px;
    margin-bottom: var(--space-3);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: var(--space-3);
  }
  .card {
    overflow: hidden;
  }
  .card img {
    width: 100%;
    height: 110px;
    object-fit: contain;
    background: var(--bg);
    display: block;
  }
  .ext {
    height: 110px;
    display: grid;
    place-items: center;
    color: var(--fg-faint);
    background: var(--bg);
    font-size: 18px;
  }
  .meta {
    padding: 8px 10px;
  }
  .fn {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row2 {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 6px;
  }
  button.danger.sm {
    padding: 3px 9px;
    font-size: 11px;
  }
  button.danger:hover {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
