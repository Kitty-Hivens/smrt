// Thin fetch client. Same-origin: the mirror serves both the panel and the
// API. credentials:'include' carries the session cookie set at login.

import type {
  AuthoringPacksListing,
  CacheInventory,
  Featured,
  Health,
  PackListing,
  ServerEntry,
  ServerListing,
} from './types';

export class ApiError extends Error {
  constructor(
    public status: number,
    public body: string,
  ) {
    super(`HTTP ${status}`);
  }
}

async function getJson<T>(path: string): Promise<T> {
  const r = await fetch(path, {
    credentials: 'include',
    headers: { Accept: 'application/json' },
  });
  if (!r.ok) throw new ApiError(r.status, await r.text().catch(() => ''));
  return (await r.json()) as T;
}

async function send(method: string, path: string, jsonBody?: unknown): Promise<void> {
  const init: RequestInit = { method, credentials: 'include' };
  if (jsonBody !== undefined) {
    init.headers = { 'Content-Type': 'application/json' };
    init.body = JSON.stringify(jsonBody);
  }
  const r = await fetch(path, init);
  if (!r.ok) throw new ApiError(r.status, await r.text().catch(() => ''));
}

export const api = {
  health: () => getJson<Health>('/v1/health'),
  packs: () => getJson<PackListing>('/v1/packs'),
  servers: () => getJson<ServerListing>('/v1/servers'),
  featured: () => getJson<Featured>('/v1/featured'),
  cacheInventory: () => getJson<CacheInventory>('/v1/cache/inventory'),
  authoringPacks: () => getJson<AuthoringPacksListing>('/v1/admin/packs'),

  // ── admin writes ──
  saveServer: (e: ServerEntry) => send('POST', '/v1/admin/servers', e),
  deleteServer: (id: string) => send('DELETE', `/v1/admin/servers/${encodeURIComponent(id)}`),
  saveFeatured: (f: Featured) => send('POST', '/v1/admin/featured', f),

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
