// Thin fetch client. Same-origin: the mirror serves both the panel and the
// API. credentials:'include' carries the session cookie set at login.

import type {
  AuditRow,
  AuthoringPacksListing,
  BuildModRow,
  BuildSummary,
  CacheInventory,
  CommunityPack,
  DeclaredAsset,
  CacheUsageListing,
  GraphData,
  GraphSlice,
  ModrinthProjectName,
  Health,
  JarDiff,
  JobResult,
  ManifestVersionsListing,
  ModDetail,
  ModHit,
  ModrinthHit,
  PackEvent,
  ModrinthVersion,
  ModSummary,
  PackConfig,
  PackListing,
  PackManifest,
  PackSummary,
  PulledPreview,
  ReleaseRow,
  ResolveReport,
  ServerEntry,
  ServerListing,
  UnassignedJar,
  UploadRow,
  UserRow,
  ValidateReport,
  VersionRow,
  Visibility,
} from './types';
import { activity } from './motion.svelte';

// The authored identity an operator sets for one cached jar: which mod, which
// release (version_number + channel), and the file's loader/mc facets. Exactly
// one of mod_id / mod_name is required (existing vs new mod).
export interface IdentityInput {
  mod_id?: number;
  mod_name?: string;
  version_number: string;
  channel: string;
  loaders: string[];
  mc_versions: string[];
  filename?: string;
}

export class ApiError extends Error {
  constructor(
    public status: number,
    public body: string,
  ) {
    super(`HTTP ${status}`);
  }
}

// A 401 mid-session means the cookie expired; let the shell bounce to login
// rather than leave the operator staring at red banners.
let onUnauthorized: (() => void) | null = null;
export function setUnauthorizedHandler(fn: () => void): void {
  onUnauthorized = fn;
}

async function toError(r: Response): Promise<ApiError> {
  if (r.status === 401) onUnauthorized?.();
  return new ApiError(r.status, await r.text().catch(() => ''));
}

// A pack config together with the revision it was read at. The revision rides
// back on the next save as `If-Match`, so a save whose base is no longer what
// is on disk is refused instead of quietly overwriting whoever saved first
// (#52). Null means "no revision known" -- an unconditional write, which is
// what a first save of a pack that has no config yet is.
export interface RevisionedConfig {
  config: PackConfig;
  rev: string | null;
}

// The entity tag as an opaque revision: quoting and the weak-validator prefix
// are transport, not identity.
function revisionOf(r: Response): string | null {
  const raw = r.headers.get('ETag');
  if (!raw) return null;
  const tag = raw.replace(/^W\//, '').replace(/^"|"$/g, '');
  return tag || null;
}

async function getJson<T>(path: string): Promise<T> {
  activity.begin();
  try {
    const r = await fetch(path, {
      credentials: 'include',
      headers: { Accept: 'application/json' },
    });
    if (!r.ok) throw await toError(r);
    return (await r.json()) as T;
  } finally {
    activity.end();
  }
}

async function send(method: string, path: string, jsonBody?: unknown): Promise<void> {
  const init: RequestInit = { method, credentials: 'include' };
  if (jsonBody !== undefined) {
    init.headers = { 'Content-Type': 'application/json' };
    init.body = JSON.stringify(jsonBody);
  }
  activity.begin();
  try {
    const r = await fetch(path, init);
    if (!r.ok) throw await toError(r);
  } finally {
    activity.end();
  }
}

async function sendRaw(
  method: string,
  path: string,
  body: ArrayBuffer,
  contentType: string,
): Promise<void> {
  const r = await fetch(path, {
    method,
    credentials: 'include',
    headers: { 'Content-Type': contentType },
    body,
  });
  if (!r.ok) throw await toError(r);
}

async function sha1Hex(buf: ArrayBuffer): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-1', buf);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

// Per-project icon cache (incl. negative results), mirroring the launcher's
// ModIconResolver. Shared across every ModIcon in the preview so a 56-mod pack
// hits each Modrinth project at most once.
const modrinthIconCache = new Map<string, string | null>();

async function resolveModrinthIcon(projectId: string): Promise<string | null> {
  const cached = modrinthIconCache.get(projectId);
  if (cached !== undefined) return cached;
  try {
    const r = await getJson<{ icon_url: string | null }>(
      `/v1/modrinth/icon?id=${encodeURIComponent(projectId)}`,
    );
    const url = r.icon_url ?? null;
    modrinthIconCache.set(projectId, url);
    return url;
  } catch {
    modrinthIconCache.set(projectId, null);
    return null;
  }
}

export const api = {
  health: () => getJson<Health>('/v1/health'),
  packs: () => getJson<PackListing>('/v1/packs'),
  community: () => getJson<CommunityPack[]>('/v1/community'),
  // fork a pack into the caller's namespace (community draft with fork_of set)
  fork: (source: string, name: string) => send('POST', '/v1/me/forks', { source, name }),
  servers: () => getJson<ServerListing>('/v1/servers'),
  cacheInventory: () => getJson<CacheInventory>('/v1/cache/inventory'),
  // admin-only: same jars, enriched with which pack/filename uses each sha1
  cacheUsage: () => getJson<CacheUsageListing>('/v1/cache/usage'),
  authoringPacks: () => getJson<AuthoringPacksListing>('/v1/authoring/packs'),
  // operator view: every pack summary incl. drafts/community that /v1/packs hides
  adminSummaries: () => getJson<PackSummary[]>('/v1/authoring/summaries'),
  // member view: the caller's own packs (built summaries + unbuilt draft ids)
  mePacks: () => getJson<PackSummary[]>('/v1/me/packs'),
  meAuthoring: () => getJson<string[]>('/v1/me/authoring'),

  // ── admin writes ──
  saveServer: (e: ServerEntry) => send('POST', '/v1/servers', e),
  deleteServer: (id: string) => send('DELETE', `/v1/servers/${encodeURIComponent(id)}`),

  // Content-addressed: hash client-side and PUT under the sha1 path. The
  // mirror re-verifies the body hashes to the claimed sha1.
  async uploadCacheJar(file: File): Promise<string> {
    const buf = await file.arrayBuffer();
    const sha1 = await sha1Hex(buf);
    await sendRaw(
      'PUT',
      `/v1/cache/${sha1.slice(0, 2)}/${sha1}.jar`,
      buf,
      'application/java-archive',
    );
    return sha1;
  },
  deleteCacheJar: (sha1: string) =>
    send('DELETE', `/v1/cache/${sha1.slice(0, 2)}/${sha1}.jar`),
  // deliberate policy block (#14): drop bytes + tombstone so it cannot be
  // re-served or re-ingested; restore lifts it
  takedownJar: (sha1: string) => send('POST', `/v1/cache/removed/${sha1}`),
  restoreJar: (sha1: string) => send('DELETE', `/v1/cache/removed/${sha1}`),

  // server-side fetch of a GitHub release asset into the cache, returning its
  // content hash; the caller adds it as a normal smrt_cache mod
  async ingestGithub(
    repo: string,
    tag: string,
    asset: string,
  ): Promise<{ sha1: string; size_bytes: number }> {
    const r = await fetch('/v1/cache/github', {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ repo, tag, asset }),
    });
    if (!r.ok) throw await toError(r);
    return (await r.json()) as { sha1: string; size_bytes: number };
  },
  removed: () => getJson<{ schema_version: number; removed: string[] }>('/v1/cache/removed'),

  // ── authoring: config, build ──
  async packConfig(id: string): Promise<RevisionedConfig> {
    activity.begin();
    try {
      const r = await fetch(`/v1/authoring/packs/${encodeURIComponent(id)}/config`, {
        credentials: 'include',
        headers: { Accept: 'application/json' },
      });
      if (!r.ok) throw await toError(r);
      return { config: (await r.json()) as PackConfig, rev: revisionOf(r) };
    } finally {
      activity.end();
    }
  },
  // Save against `baseRev`: the mirror refuses the write with 409 when the
  // stored config has moved on since. Passing null writes unconditionally --
  // creating a config that does not exist yet, or a deliberate overwrite.
  // Returns what the mirror stored -- the config with the fill's dependencies in
  // it, and the revision to edit on from.
  async savePackConfig(
    id: string,
    cfg: PackConfig,
    baseRev: string | null,
  ): Promise<RevisionedConfig> {
    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (baseRev) headers['If-Match'] = `"${baseRev}"`;
    activity.begin();
    try {
      const r = await fetch(`/v1/authoring/packs/${encodeURIComponent(id)}/config`, {
        method: 'PUT',
        credentials: 'include',
        headers,
        body: JSON.stringify(cfg),
      });
      if (!r.ok) throw await toError(r);
      return { config: (await r.json()) as PackConfig, rev: revisionOf(r) };
    } finally {
      activity.end();
    }
  },
  // overwrite the config with one reconstructed from a published build; returns it
  async revertPackConfig(id: string, version: string): Promise<RevisionedConfig> {
    const r = await fetch(
      `/v1/authoring/packs/${encodeURIComponent(id)}/config/revert?version=${encodeURIComponent(version)}`,
      { method: 'POST', credentials: 'include' },
    );
    if (!r.ok) throw await toError(r);
    return { config: (await r.json()) as PackConfig, rev: revisionOf(r) };
  },
  async buildPack(
    id: string,
    opts?: {
      dryRun?: boolean;
      packVersion?: string;
      channel?: 'release' | 'beta' | 'alpha';
      changelog?: string;
      // publish over a pre-publish check that says the pack cannot start;
      // recorded on the build, in the job log and in the audit trail
      overrideChecks?: boolean;
    },
  ): Promise<{ job_id: string }> {
    const q = new URLSearchParams();
    if (opts?.dryRun) q.set('dry_run', 'true');
    if (opts?.overrideChecks) q.set('override_checks', 'true');
    if (opts?.packVersion) q.set('pack_version', opts.packVersion);
    if (opts?.channel) q.set('channel', opts.channel);
    const qs = q.toString();
    const changelog = opts?.changelog?.trim();
    const r = await fetch(`/v1/authoring/packs/${encodeURIComponent(id)}/build${qs ? `?${qs}` : ''}`, {
      method: 'POST',
      credentials: 'include',
      ...(changelog
        ? {
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ changelog }),
          }
        : {}),
    });
    if (!r.ok) throw await toError(r);
    return (await r.json()) as { job_id: string };
  },
  jobEventsUrl: (jobId: string) => `/v1/jobs/${encodeURIComponent(jobId)}/events`,
  // What is happening to a pack while it is open: who else is in it, and that it
  // moved. Subscribing is also how the mirror learns you are here.
  packEventsUrl: (id: string) => `/v1/authoring/packs/${encodeURIComponent(id)}/events`,

  // The pack's merge document (#115). Binary both ways: these are CRDT updates,
  // not configs, and base64 would only be the room's problem (server-sent events
  // being a text protocol).
  async packDocState(id: string): Promise<Uint8Array> {
    const r = await fetch(`/v1/authoring/packs/${encodeURIComponent(id)}/doc`, {
      credentials: 'include',
    });
    if (!r.ok) throw await toError(r);
    return new Uint8Array(await r.arrayBuffer());
  },
  async sendPackDoc(id: string, update: Uint8Array): Promise<void> {
    const r = await fetch(`/v1/authoring/packs/${encodeURIComponent(id)}/doc`, {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/octet-stream' },
      body: update as unknown as BodyInit,
    });
    if (!r.ok) throw await toError(r);
  },
  jobStatus: (jobId: string) => getJson<JobResult>(`/v1/jobs/${encodeURIComponent(jobId)}`),

  // ── published manifest (preview baseline + version diff) ──
  manifest: (id: string) => getJson<PackManifest>(`/v1/packs/${encodeURIComponent(id)}/manifest`),
  manifestVersions: (id: string) =>
    getJson<ManifestVersionsListing>(`/v1/packs/${encodeURIComponent(id)}/manifest/versions`),
  manifestVersion: (id: string, version: string) =>
    getJson<PackManifest>(
      `/v1/packs/${encodeURIComponent(id)}/manifest/${encodeURIComponent(version)}`,
    ),

  // What saving this config would pull in, asked before the save. Read-only:
  // the mirror runs the real fill on a copy and writes nothing.
  async previewDependencies(id: string, cfg: PackConfig): Promise<PulledPreview[]> {
    const r = await fetch(
      `/v1/authoring/packs/${encodeURIComponent(id)}/dependency-preview`,
      {
        method: 'POST',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(cfg),
      },
    );
    if (!r.ok) throw await toError(r);
    return (await r.json()) as PulledPreview[];
  },

  // ── resolve the saved config against the registry dependency graph ──
  // the pack's own relation graph: its mods, wired by what its shipped artifacts
  // declare. A dangling target is a requirement this pack does not carry.
  packGraph: (id: string) =>
    getJson<GraphData>(`/v1/authoring/packs/${encodeURIComponent(id)}/graph`),
  resolvePack: (id: string) =>
    getJson<ResolveReport>(`/v1/authoring/packs/${encodeURIComponent(id)}/resolve`),

  // ── validate a config against an SC archive ──
  async validatePack(id: string, file: File): Promise<ValidateReport> {
    const buf = await file.arrayBuffer();
    const r = await fetch(`/v1/authoring/packs/${encodeURIComponent(id)}/validate`, {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/zip' },
      body: buf,
    });
    if (!r.ok) throw await toError(r);
    return (await r.json()) as ValidateReport;
  },

  // ── bootstrap + pack static assets ──
  async bootstrapPack(
    id: string,
    params: {
      minecraft_version: string;
      loader_version: string;
      display_name?: string;
      tagline?: string;
      loader_name?: string;
      java_major?: number;
    },
    file: File,
  ): Promise<{ job_id: string }> {
    const q = new URLSearchParams();
    q.set('minecraft_version', params.minecraft_version);
    q.set('loader_version', params.loader_version);
    if (params.display_name) q.set('display_name', params.display_name);
    if (params.tagline) q.set('tagline', params.tagline);
    if (params.loader_name) q.set('loader_name', params.loader_name);
    if (params.java_major != null) q.set('java_major', String(params.java_major));
    const buf = await file.arrayBuffer();
    const r = await fetch(`/v1/authoring/packs/${encodeURIComponent(id)}/bootstrap?${q}`, {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/zip' },
      body: buf,
    });
    if (!r.ok) throw await toError(r);
    return (await r.json()) as { job_id: string };
  },
  packStatic: (id: string) =>
    getJson<{ schema_version: number; pack_id: string; files: string[] }>(
      `/v1/authoring/packs/${encodeURIComponent(id)}/static`,
    ),
  async uploadStatic(id: string, relPath: string, file: File): Promise<void> {
    const buf = await file.arrayBuffer();
    const enc = relPath.split('/').map(encodeURIComponent).join('/');
    await sendRaw(
      'PUT',
      `/v1/authoring/packs/${encodeURIComponent(id)}/static/${enc}`,
      buf,
      file.type || 'application/octet-stream',
    );
  },
  deleteStatic(id: string, relPath: string): Promise<void> {
    const enc = relPath.split('/').map(encodeURIComponent).join('/');
    return send('DELETE', `/v1/authoring/packs/${encodeURIComponent(id)}/static/${enc}`);
  },
  staticUrl(id: string, relPath: string): string {
    const enc = relPath.split('/').map(encodeURIComponent).join('/');
    return `/v1/packs/${encodeURIComponent(id)}/static/${enc}`;
  },

  // One search over both places a mod can come from: the mirror's registry and
  // Modrinth, merged and ranked against the pack (#101). `pack` is what makes a
  // bridged verdict possible -- whether the connector is already in the pack.
  searchMods: (
    q: string,
    opts: { mc?: string; loader?: string; pack?: string; limit?: number } = {},
  ) => {
    const p = new URLSearchParams({ q });
    if (opts.mc) p.set('mc', opts.mc);
    if (opts.loader) p.set('loader', opts.loader);
    if (opts.pack) p.set('pack', opts.pack);
    if (opts.limit) p.set('limit', String(opts.limit));
    return getJson<ModHit[]>(`/v1/search/mods?${p}`);
  },

  // ── Modrinth search-to-add ──
  modrinthSearch: (q: string, mc?: string, type?: string) =>
    getJson<ModrinthHit[]>(
      `/v1/modrinth/search?q=${encodeURIComponent(q)}${mc ? `&mc=${encodeURIComponent(mc)}` : ''}${type ? `&type=${encodeURIComponent(type)}` : ''}`,
    ),
  modrinthVersions: (id: string, mc?: string) =>
    getJson<ModrinthVersion[]>(
      `/v1/modrinth/versions?id=${encodeURIComponent(id)}${mc ? `&mc=${encodeURIComponent(mc)}` : ''}`,
    ),
  // Same per-project lookup the launcher's ModIconResolver does; cached.
  modrinthIcon: (projectId: string) => resolveModrinthIcon(projectId),

  // ── registry browser (the mirror's own mods + builds) ──
  registryMods: (q?: string, loader?: string, mc?: string) => {
    const p = new URLSearchParams();
    if (q) p.set('q', q);
    if (loader) p.set('loader', loader);
    if (mc) p.set('mc', mc);
    const qs = p.toString();
    return getJson<ModSummary[]>(`/v1/registry/mods${qs ? `?${qs}` : ''}`);
  },
  registryModVersions: (modId: number) =>
    getJson<VersionRow[]>(`/v1/registry/mod-versions/${modId}`),
  // a mod's files grouped by release (version node) for the management view
  modReleases: (modId: number) =>
    getJson<ReleaseRow[]>(`/v1/registry/mod-releases/${modId}`),
  // public per-mod read model behind the mod page (guest-accessible). `ref` is a
  // numeric mod id or a `sha1:<hash>` artifact reference.
  modDetail: (ref: number | string) =>
    getJson<ModDetail>(`/v1/mods/${encodeURIComponent(ref)}`),
  // jars on disk with no identity yet -- the "needs identity" bucket
  unassigned: () => getJson<UnassignedJar[]>('/v1/registry/unassigned'),
  // set a cached jar's mod + release + facets (authored, survives re-harvest)
  authorFileIdentity: (sha1: string, body: IdentityInput) =>
    send('PUT', `/v1/registry/files/${sha1}/identity`, body),
  renameMod: (modId: number, body: { name?: string; slug?: string }) =>
    send('PUT', `/v1/registry/mod-meta/${modId}`, body),
  editRelease: (releaseId: number, body: { version_number?: string; channel?: string }) =>
    send('PUT', `/v1/registry/releases/${releaseId}`, body),
  // merge one mod identity into another (surviving into_mod_id); debug-gated
  mergeMods: (fromModId: number, intoModId: number) =>
    send('POST', '/v1/registry/merge', { from_mod_id: fromModId, into_mod_id: intoModId }),
  // what a self-hosted jar changed vs its genuine Modrinth counterpart
  repackDiff: (sha1: string) => getJson<JarDiff>(`/v1/registry/files/${sha1}/repack-diff`),
  // The dependency/conflict graph, narrowed to one (mc, loader) world. Unnarrowed
  // it unions every version of every mod, which only reads once the registry holds
  // a single world (#49).
  graph: (mc?: string, loader?: string) => {
    const p = new URLSearchParams();
    if (mc) p.set('mc', mc);
    if (loader) p.set('loader', loader);
    const qs = p.toString();
    return getJson<GraphData>(`/v1/registry/graph${qs ? `?${qs}` : ''}`);
  },
  // the (mc, loader) worlds the registry holds, busiest first
  graphSlices: () => getJson<GraphSlice[]>('/v1/registry/graph/slices'),
  // resolve external `modrinth:<id>` graph leaves to real names (cache-first)
  modrinthNames: (ids: string[]) =>
    getJson<ModrinthProjectName[]>(
      `/v1/registry/modrinth-names?ids=${encodeURIComponent(ids.join(','))}`,
    ),
  // author or remove one graph edge (node editor); debug-gated
  authorRelation: (body: {
    from_mod_id: number;
    target_modid: string;
    kind: string;
    remove?: boolean;
  }) => send('POST', '/v1/registry/relations', body),
  registryBuilds: () => getJson<BuildSummary[]>('/v1/registry/builds'),
  registryBuildMods: (packId: string, packVersion: string) =>
    getJson<BuildModRow[]>(
      `/v1/registry/builds/${encodeURIComponent(packId)}/${encodeURIComponent(packVersion)}`,
    ),
  registryBuildAssets: (packId: string, packVersion: string) =>
    getJson<DeclaredAsset[]>(
      `/v1/registry/builds/${encodeURIComponent(packId)}/${encodeURIComponent(packVersion)}/assets`,
    ),

  listUsers: () => getJson<UserRow[]>('/v1/users'),
  auditLog: () => getJson<AuditRow[]>('/v1/audit'),
  setUserRole: (uid: number, role: string) =>
    send('POST', `/v1/users/${uid}/role`, { role }),
  setVisibility: (id: string, visibility: Visibility) =>
    send('PUT', `/v1/authoring/packs/${encodeURIComponent(id)}/visibility`, { visibility }),
  deletePack: (id: string) => send('DELETE', `/v1/authoring/packs/${encodeURIComponent(id)}`),

  // ── upload moderation ──
  // operator queue
  pendingUploads: () => getJson<UploadRow[]>('/v1/uploads'),
  approveUpload: (uploadId: number) => send('POST', `/v1/uploads/${uploadId}/approve`),
  rejectUpload: (uploadId: number, note: string) =>
    send('POST', `/v1/uploads/${uploadId}/reject`, { note }),
  // member: upload a self-hosted jar for a community pack, and see own uploads
  myUploads: () => getJson<UploadRow[]>('/v1/me/uploads'),
  async uploadJar(
    packId: string,
    file: File,
    opts?: { maintainer?: string; force?: boolean },
  ): Promise<UploadRow> {
    const buf = await file.arrayBuffer();
    const q = new URLSearchParams({ filename: file.name });
    if (opts?.maintainer) q.set('maintainer', opts.maintainer);
    if (opts?.force) q.set('force', 'true');
    const r = await fetch(`/v1/me/packs/${encodeURIComponent(packId)}/uploads?${q}`, {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/java-archive' },
      body: buf,
    });
    if (!r.ok) throw await toError(r);
    return (await r.json()) as UploadRow;
  },

  async me(): Promise<{
    uid: number;
    login: string;
    role: string;
    accepted_terms: boolean;
  } | null> {
    const r = await fetch('/v1/me', { credentials: 'include' });
    return r.ok ? r.json() : null;
  },
  acceptTerms: () => send('POST', '/v1/me/accept-terms'),
  // The admin token no longer authenticates a human. A valid one comes back 410
  // so the panel can say it's deprecated; anything else is a plain rejection.
  async login(token: string): Promise<'deprecated' | 'rejected'> {
    const r = await fetch('/v1/auth/login', {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token }),
    });
    return r.status === 410 ? 'deprecated' : 'rejected';
  },
  async logout(): Promise<void> {
    await fetch('/v1/auth/logout', { method: 'POST', credentials: 'include' });
  },
};
