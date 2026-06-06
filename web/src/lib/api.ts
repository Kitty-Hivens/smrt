// Thin fetch client. Same-origin: the mirror serves both the panel and the
// API. credentials:'include' carries the session cookie set at login.

import type {
  AuthoringPacksListing,
  CacheInventory,
  CacheUsageListing,
  Featured,
  Curator,
  Health,
  JobResult,
  ManifestVersionsListing,
  ModrinthHit,
  ModrinthVersion,
  PackConfig,
  PackListing,
  PackManifest,
  ServerEntry,
  ServerListing,
  ValidateReport,
} from './types';

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

async function getJson<T>(path: string): Promise<T> {
  const r = await fetch(path, {
    credentials: 'include',
    headers: { Accept: 'application/json' },
  });
  if (!r.ok) throw await toError(r);
  return (await r.json()) as T;
}

async function send(method: string, path: string, jsonBody?: unknown): Promise<void> {
  const init: RequestInit = { method, credentials: 'include' };
  if (jsonBody !== undefined) {
    init.headers = { 'Content-Type': 'application/json' };
    init.body = JSON.stringify(jsonBody);
  }
  const r = await fetch(path, init);
  if (!r.ok) throw await toError(r);
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

async function getText(path: string): Promise<string> {
  const r = await fetch(path, { credentials: 'include' });
  if (!r.ok) throw await toError(r);
  return r.text();
}

async function putText(path: string, text: string): Promise<void> {
  const r = await fetch(path, {
    method: 'PUT',
    credentials: 'include',
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
    body: text,
  });
  if (!r.ok) throw await toError(r);
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
      `/v1/admin/modrinth/icon?id=${encodeURIComponent(projectId)}`,
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
  servers: () => getJson<ServerListing>('/v1/servers'),
  featured: () => getJson<Featured>('/v1/featured'),
  cacheInventory: () => getJson<CacheInventory>('/v1/cache/inventory'),
  // admin-only: same jars, enriched with which pack/filename uses each sha1
  cacheUsage: () => getJson<CacheUsageListing>('/v1/admin/cache/inventory'),
  authoringPacks: () => getJson<AuthoringPacksListing>('/v1/admin/packs'),

  // ── admin writes ──
  saveServer: (e: ServerEntry) => send('POST', '/v1/admin/servers', e),
  deleteServer: (id: string) => send('DELETE', `/v1/admin/servers/${encodeURIComponent(id)}`),
  saveFeatured: (f: Featured) => send('POST', '/v1/admin/featured', f),

  // Content-addressed: hash client-side and PUT under the sha1 path. The
  // mirror re-verifies the body hashes to the claimed sha1.
  async uploadCacheJar(file: File): Promise<string> {
    const buf = await file.arrayBuffer();
    const sha1 = await sha1Hex(buf);
    await sendRaw(
      'PUT',
      `/v1/admin/cache/${sha1.slice(0, 2)}/${sha1}.jar`,
      buf,
      'application/java-archive',
    );
    return sha1;
  },
  deleteCacheJar: (sha1: string) =>
    send('DELETE', `/v1/admin/cache/${sha1.slice(0, 2)}/${sha1}.jar`),
  removed: () => getJson<{ schema_version: number; removed: string[] }>('/v1/admin/cache/removed'),

  // ── authoring: config, curator, build ──
  packConfig: (id: string) =>
    getJson<PackConfig>(`/v1/admin/packs/${encodeURIComponent(id)}/config`),
  savePackConfig: (id: string, cfg: PackConfig) =>
    send('PUT', `/v1/admin/packs/${encodeURIComponent(id)}/config`, cfg),
  curator: (id: string) => getText(`/v1/admin/packs/${encodeURIComponent(id)}/curator`),
  saveCurator: (id: string, text: string) =>
    putText(`/v1/admin/packs/${encodeURIComponent(id)}/curator`, text),
  curatorStructured: (id: string) =>
    getJson<Curator>(`/v1/admin/packs/${encodeURIComponent(id)}/curator/structured`),
  saveCuratorStructured: (id: string, c: Curator) =>
    send('PUT', `/v1/admin/packs/${encodeURIComponent(id)}/curator/structured`, c),
  async buildPack(
    id: string,
    opts?: { dryRun?: boolean; packVersion?: string },
  ): Promise<{ job_id: string }> {
    const q = new URLSearchParams();
    if (opts?.dryRun) q.set('dry_run', 'true');
    if (opts?.packVersion) q.set('pack_version', opts.packVersion);
    const qs = q.toString();
    const r = await fetch(`/v1/admin/packs/${encodeURIComponent(id)}/build${qs ? `?${qs}` : ''}`, {
      method: 'POST',
      credentials: 'include',
    });
    if (!r.ok) throw await toError(r);
    return (await r.json()) as { job_id: string };
  },
  jobEventsUrl: (jobId: string) => `/v1/admin/jobs/${encodeURIComponent(jobId)}/events`,
  jobStatus: (jobId: string) => getJson<JobResult>(`/v1/admin/jobs/${encodeURIComponent(jobId)}`),

  // ── published manifest (preview baseline + version diff) ──
  manifest: (id: string) => getJson<PackManifest>(`/v1/packs/${encodeURIComponent(id)}/manifest`),
  manifestVersions: (id: string) =>
    getJson<ManifestVersionsListing>(`/v1/packs/${encodeURIComponent(id)}/manifest/versions`),
  manifestVersion: (id: string, version: string) =>
    getJson<PackManifest>(
      `/v1/packs/${encodeURIComponent(id)}/manifest/${encodeURIComponent(version)}`,
    ),

  // ── validate a config against an SC archive ──
  async validatePack(id: string, file: File): Promise<ValidateReport> {
    const buf = await file.arrayBuffer();
    const r = await fetch(`/v1/admin/packs/${encodeURIComponent(id)}/validate`, {
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
    const r = await fetch(`/v1/admin/packs/${encodeURIComponent(id)}/bootstrap?${q}`, {
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
      `/v1/admin/packs/${encodeURIComponent(id)}/static`,
    ),
  async uploadStatic(id: string, relPath: string, file: File): Promise<void> {
    const buf = await file.arrayBuffer();
    const enc = relPath.split('/').map(encodeURIComponent).join('/');
    await sendRaw(
      'PUT',
      `/v1/admin/packs/${encodeURIComponent(id)}/static/${enc}`,
      buf,
      file.type || 'application/octet-stream',
    );
  },
  deleteStatic(id: string, relPath: string): Promise<void> {
    const enc = relPath.split('/').map(encodeURIComponent).join('/');
    return send('DELETE', `/v1/admin/packs/${encodeURIComponent(id)}/static/${enc}`);
  },
  staticUrl(id: string, relPath: string): string {
    const enc = relPath.split('/').map(encodeURIComponent).join('/');
    return `/v1/packs/${encodeURIComponent(id)}/static/${enc}`;
  },

  // ── Modrinth search-to-add ──
  modrinthSearch: (q: string, mc?: string) =>
    getJson<ModrinthHit[]>(
      `/v1/admin/modrinth/search?q=${encodeURIComponent(q)}${mc ? `&mc=${encodeURIComponent(mc)}` : ''}`,
    ),
  modrinthVersions: (id: string, mc?: string) =>
    getJson<ModrinthVersion[]>(
      `/v1/admin/modrinth/versions?id=${encodeURIComponent(id)}${mc ? `&mc=${encodeURIComponent(mc)}` : ''}`,
    ),
  // Same per-project lookup the launcher's ModIconResolver does; cached.
  modrinthIcon: (projectId: string) => resolveModrinthIcon(projectId),

  async session(): Promise<boolean> {
    const r = await fetch('/admin/api/session', { credentials: 'include' });
    return r.ok;
  },
  async login(token: string): Promise<boolean> {
    const r = await fetch('/admin/api/login', {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token }),
    });
    return r.ok;
  },
  async logout(): Promise<void> {
    await fetch('/admin/api/logout', { method: 'POST', credentials: 'include' });
  },
};
