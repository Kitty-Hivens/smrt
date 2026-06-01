// Wire DTOs the panel reads. Hand-written for the Phase 2 shell; Phase 4
// replaces these with ts-rs codegen from the mirror's Rust structs so the
// two never drift.

export interface Health {
  schema_version: number;
  status: string;
  version: string;
}

export interface PackSummary {
  pack_id: string;
  display_name: string;
  tagline: string;
  minecraft_version: string;
  latest_pack_version: string;
  tags: string[];
  featured?: boolean;
  icon_url?: string | null;
  banner_url?: string | null;
  gallery_urls?: string[];
  description_md?: string | null;
}

export interface PackListing {
  schema_version: number;
  generated_at: string;
  packs: PackSummary[];
}

export interface ServerEntry {
  schema_version: number;
  server_id: string;
  pack_id: string;
  display_name: string;
  tagline: string;
  description_md: string;
  banner_url: string;
  gallery_urls?: string[];
  tags?: string[];
  discord_url?: string | null;
  website_url?: string | null;
  owner_display: string;
  motd_override?: string | null;
  founded_at?: string | null;
  featured?: boolean;
}

export interface ServerListing {
  schema_version: number;
  generated_at: string;
  servers: ServerEntry[];
}

export interface Featured {
  schema_version: number;
  generated_at: string;
  featured_servers: string[];
  featured_packs: string[];
}

export interface CacheInventoryEntry {
  sha1: string;
  size_bytes: number;
}

export interface CacheInventory {
  schema_version: number;
  generated_at: string;
  entries: CacheInventoryEntry[];
}

export interface AuthoringPacksListing {
  schema_version: number;
  packs: string[];
}

// ── Authoring config (the editable PackConfig) ──

export interface LoaderSpec {
  name: string;
  version: string;
}

export interface DisplayMeta {
  name?: string | null;
  description?: string | null;
  category?: string | null;
  incompatible_with?: string[];
  license?: string | null;
  url?: string | null;
  icon_url?: string | null;
  role?: string | null;
}

export type SourceDecl =
  | { type: 'modrinth'; project_id: string; version_id: string }
  | { type: 'smrt_cache'; sha1: string }
  | { type: 'smrt_static'; rel_path: string };

export interface DeclaredMod {
  filename: string;
  required?: boolean;
  default_enabled?: boolean;
  source: SourceDecl;
  display?: DisplayMeta | null;
  note?: string | null;
}

export interface DeclaredAsset {
  dest: string;
  required?: boolean;
  source: SourceDecl;
  display?: DisplayMeta | null;
  note?: string | null;
}

export interface PackConfig {
  pack_id: string;
  display_name: string;
  tagline: string;
  minecraft_version: string;
  loader: LoaderSpec;
  java_major: number;
  tags?: string[];
  featured?: boolean;
  mods: DeclaredMod[];
  assets?: DeclaredAsset[];
}

export type JobStatus = 'running' | 'done' | 'failed';

export interface ModrinthHit {
  project_id: string;
  slug: string;
  title: string;
  description: string;
  icon_url?: string | null;
}

export interface ModrinthVersion {
  id: string;
  project_id: string;
  version_number: string;
  game_versions: string[];
  loaders: string[];
}
