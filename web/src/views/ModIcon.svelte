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
    size = 34,
  }: { name: string; iconUrl?: string | null; source: Source | SourceDecl; size?: number } =
    $props();

  const avatar = $derived(letterAvatar(name));
  const explicit = $derived(iconUrl?.trim() || null);
  // Modrinth project icon, resolved lazily when there is no explicit icon_url.
  let modrinth = $state<string | null>(null);
  let broken = $state(false);
  // a self-hosted jar's own embedded icon, extracted + served by the mirror; the
  // <img> onerror falls back to the letter avatar when the jar carries none (404)
  const cacheIcon = $derived(
    source.type === 'smrt_cache' && 'sha1' in source && source.sha1
      ? `/v1/admin/cache/icon/${source.sha1}`
      : null,
  );
  const src = $derived(broken ? null : (explicit ?? modrinth ?? cacheIcon));

  // Mirror ModIconResolver: only fall back to the project icon when no explicit
  // icon_url is set and the source is Modrinth (cached in the api layer). This
  // also re-runs when `source` changes -- a list row may be reused for a
  // different mod (sort / re-point), so reset the resolved/broken state first
  // instead of carrying the previous mod's icon over.
  $effect(() => {
    void source; // re-run on identity change
    broken = false;
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
    onerror={() => (broken = true)}
  />
{:else}
  <span
    class="mi avatar"
    style="width:{size}px;height:{size}px;background:{avatar.color};font-size:{Math.round(
      size * 0.4,
    )}px"
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
</style>
