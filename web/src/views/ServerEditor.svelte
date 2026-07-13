<script lang="ts">
  import { untrack } from 'svelte';
  import { api, ApiError } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import Section from './ui/Section.svelte';
  import Field from './ui/Field.svelte';
  import type { ServerEntry } from '../lib/types';

  let {
    initial,
    packIds,
    onSaved,
    onCancel,
  }: {
    initial: ServerEntry | null;
    packIds: string[];
    onSaved: () => void;
    onCancel: () => void;
  } = $props();

  const isNew = $derived(initial === null);

  // One-shot working copy; the parent remounts this editor per row via {#key},
  // so capturing the initial value here is intentional (hence untrack).
  let f = $state<ServerEntry>(
    untrack(() =>
      initial
        ? {
            ...initial,
            tags: [...(initial.tags ?? [])],
            gallery_urls: [...(initial.gallery_urls ?? [])],
          }
        : {
            schema_version: 2,
            server_id: '',
            pack_id: packIds[0] ?? '',
            display_name: '',
            tagline: '',
            description_md: '',
            banner_url: '',
            gallery_urls: [],
            tags: [],
            owner_display: '',
            featured: false,
          },
    ),
  );
  let tagsStr = $state(untrack(() => (initial?.tags ?? []).join(', ')));
  let busy = $state(false);
  let err = $state('');

  async function save(e: Event) {
    e.preventDefault();
    busy = true;
    err = '';
    const payload: ServerEntry = {
      ...$state.snapshot(f),
      tags: tagsStr
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean),
    };
    // Empty optional strings drop out (the field is optional; the mirror treats
    // an absent key as None via skip_serializing_if).
    for (const k of ['discord_url', 'website_url', 'motd_override', 'founded_at'] as const) {
      if (!payload[k]) payload[k] = undefined;
    }
    try {
      await api.saveServer(payload);
      onSaved();
    } catch (x) {
      err = x instanceof ApiError ? `${x.status} ${x.body}` : String(x);
    } finally {
      busy = false;
    }
  }
</script>

<form class="editor" onsubmit={save}>
  <div class="hd">
    <h2 class="ttl">{isNew ? t('servers.new') : t('se.edit', { id: f.server_id })}</h2>
    <div class="spacer"></div>
    <button type="button" onclick={onCancel}>{t('dialog.cancel')}</button>
    <button class="primary" type="submit" disabled={busy || !f.server_id || !f.pack_id}>
      {busy ? t('se.saving') : isNew ? t('se.create') : t('se.save')}
    </button>
  </div>
  {#if err}<div class="err mono">{err}</div>{/if}

  <Section title={t('pe.basics')}>
    <div class="grid">
      <Field label={t('se.serverId')} hint={t('se.serverIdHint')}>
        <input bind:value={f.server_id} disabled={!isNew} placeholder="main" />
      </Field>
      <Field label={t('packs.col.pack')}>
        <input bind:value={f.pack_id} list="packids" placeholder="Industrial" />
        <datalist id="packids">{#each packIds as p}<option value={p}></option>{/each}</datalist>
      </Field>
      <Field label={t('pe.displayName')}>
        <input bind:value={f.display_name} />
      </Field>
      <Field label={t('servers.col.owner')}>
        <input bind:value={f.owner_display} />
      </Field>
      <label class="chk">
        <input type="checkbox" bind:checked={f.featured} />
        {t('pe.featured')}
      </label>
    </div>
  </Section>

  <Section title={t('se.card')}>
    <div class="grid">
      <Field label={t('pe.tagline')} wide>
        <input bind:value={f.tagline} />
      </Field>
      <Field label={t('se.banner')} wide>
        <input bind:value={f.banner_url} placeholder="https://..." />
      </Field>
      <Field label={t('pe.tags')} hint={t('pe.tagsHint')} wide>
        <input bind:value={tagsStr} placeholder="tech, economy" />
      </Field>
      <Field label={t('se.description')} hint={t('se.descHint')} wide>
        <textarea rows="5" bind:value={f.description_md}></textarea>
      </Field>
    </div>
  </Section>

  <Section title={t('se.links')}>
    <div class="grid">
      <Field label={t('se.discord')}>
        <input bind:value={f.discord_url} placeholder="https://discord.gg/..." />
      </Field>
      <Field label={t('se.website')}>
        <input bind:value={f.website_url} placeholder="https://..." />
      </Field>
    </div>
  </Section>
</form>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    margin-bottom: var(--space-4);
  }
  .hd {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .ttl {
    font-size: 15px;
  }
  .spacer {
    flex: 1;
  }
  .err {
    color: var(--danger);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: var(--radius-sm);
    padding: var(--space-3) var(--space-4);
    font-size: 12px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3) var(--space-4);
  }
  .chk {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    color: var(--fg);
    grid-column: 1 / -1;
  }
  @media (max-width: 560px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
</style>
