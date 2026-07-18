<script lang="ts">
  import { api } from '../lib/api';
  import { letterAvatar } from '../lib/preview';
  import type { Source, SourceDecl } from '../lib/types';

  // accepts the wire Source (preview) or the authoring SourceDecl (editor) -- both
  // carry `type`, and the modrinth variant of each carries `project_id`, which is
  // all this component reads
  let {
    name,
    iconUrl = null,
    source,
    sha1 = null,
    size = 34,
    mono = false,
  }: {
    name: string;
    iconUrl?: string | null;
    source: Source | SourceDecl;
    // the artifact's sha1 where the caller knows it (manifest entries, registry
    // rows): unlocks the jar-embedded icon fallback for Modrinth-sourced mods
    // whose bytes the mirror caches anyway
    sha1?: string | null;
    size?: number;
    // monochrome letter fallback for the control panel; the launcher preview
    // keeps the hashed colour
    mono?: boolean;
  } = $props();

  const avatar = $derived(letterAvatar(name));
  const explicit = $derived(iconUrl?.trim() || null);
  // Modrinth project icon, resolved lazily when there is no explicit icon_url.
  let modrinth = $state<string | null>(null);
  // index into the candidate chain; a load error advances to the next candidate
  // instead of giving up, so an unreachable Modrinth CDN degrades to the jar's
  // own embedded icon (served by the mirror) before the letter avatar
  let failed = $state(0);
  const cacheSha = $derived(
    source.type === 'smrt_cache' && 'sha1' in source && source.sha1 ? source.sha1 : sha1,
  );
  const candidates = $derived(
    [explicit, modrinth, cacheSha ? `/v1/cache/icon/${cacheSha}` : null].filter(
      (c): c is string => !!c,
    ),
  );
  const src = $derived(candidates[failed] ?? null);

  // Mirror ModIconResolver: only fall back to the project icon when no explicit
  // icon_url is set and the source is Modrinth (cached in the api layer). This
  // also re-runs when `source` changes -- a list row may be reused for a
  // different mod (sort / re-point), so reset the resolved/failed state first
  // instead of carrying the previous mod's icon over.
  $effect(() => {
    void source; // re-run on identity change
    failed = 0;
    modrinth = null;
    if (explicit || source.type !== 'modrinth') return;
    let alive = true;
    void api.modrinthIcon(source.project_id).then((url) => {
      if (alive && url) modrinth = url;
    });
    return () => {
      alive = false;
    };
  });
</script>

{#if src}
  <img
    class="mi"
    style="width:{size}px;height:{size}px"
    {src}
    alt={name}
    loading="lazy"
    onerror={() => (failed = failed + 1)}
  />
{:else}
  <span
    class="mi avatar"
    class:mono
    style="width:{size}px;height:{size}px;font-size:{Math.round(size * 0.4)}px{mono
      ? ''
      : `;background:${avatar.color}`}"
    aria-label={name}>{avatar.initials}</span
  >
{/if}

<style>
  .mi {
    flex: none;
    border-radius: 6px;
    object-fit: cover;
  }
  .avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    font-weight: 600;
    letter-spacing: -0.02em;
    user-select: none;
  }
  .avatar.mono {
    background: var(--panel-3);
    color: var(--fg-dim);
  }
</style>
