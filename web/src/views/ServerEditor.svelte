<script lang="ts">
  import { untrack } from 'svelte';
  import { api, ApiError } from '../lib/api';
  import { notifyFail } from '../lib/toasts.svelte';
  import { t } from '../lib/i18n.svelte';
  import Section from './ui/Section.svelte';
  import { idError, requiredError, say, urlError } from '../lib/validate';
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

  async function save(e: Event) {
    e.preventDefault();
    busy = true;
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
      notifyFail(x);
    } finally {
      busy = false;
    }
  }

  // Every rule the mirror would apply, in one place: the submit and the fields
  // read the same verdicts, so a disabled button always has a field explaining
  // itself.
  const invalid = $derived(
    !!idError(f.server_id) ||
      !!requiredError(f.pack_id) ||
      !!urlError(f.banner_url ?? '') ||
      !!urlError(f.discord_url ?? '') ||
      !!urlError(f.website_url ?? ''),
  );
</script>

<!-- the submit is refused for exactly what the mirror would refuse, so the
     button and the fields never disagree -->
<form class="editor" onsubmit={save}>
  <div class="hd">
    <h2 class="ttl">{isNew ? t('servers.new') : t('se.edit', { id: f.server_id })}</h2>
    <div class="spacer"></div>
    <button type="button" onclick={onCancel}>{t('dialog.cancel')}</button>
    <button class="primary" type="submit" disabled={busy || invalid}>
      {busy ? t('se.saving') : isNew ? t('se.create') : t('se.save')}
    </button>
  </div>

  <Section title={t('pe.basics')}>
    <div class="grid">
      <Field label={t('se.serverId')} hint={t('se.serverIdHint')} error={say(idError(f.server_id))}>
        <input bind:value={f.server_id} disabled={!isNew} placeholder="main" />
      </Field>
      <Field label={t('packs.col.pack')} error={say(requiredError(f.pack_id))}>
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
      <Field label={t('se.banner')} wide error={say(urlError(f.banner_url ?? ''))}>
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
      <Field label={t('se.discord')} error={say(urlError(f.discord_url ?? ''))}>
        <input bind:value={f.discord_url} placeholder="https://discord.gg/..." />
      </Field>
      <Field label={t('se.website')} error={say(urlError(f.website_url ?? ''))}>
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
    font-size: var(--fs-lg);
  }
  .spacer {
    flex: 1;
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
    font-size: var(--fs-md);
    color: var(--fg);
    grid-column: 1 / -1;
  }
  @container view (max-width: 560px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
</style>
