// Wire DTOs are generated from the Rust structs by ts-rs -- see bindings/,
// regenerated with `TS_RS_EXPORT_DIR=web/src/lib cargo test` from the smrt
// crate root. This barrel re-exports them so the panel imports stay stable.
// Operational / external types that have no Rust counterpart stay hand-written
// at the bottom.

import type { DryRun } from './bindings/DryRun';

export type { DryRun };
export type { Health } from './bindings/Health';
export type { PackSummary } from './bindings/PackSummary';
export type { PackListing } from './bindings/PackListing';
export type { CommunityPack } from './bindings/CommunityPack';
export type { ManifestVersionsListing } from './bindings/ManifestVersionsListing';
export type { AuthoringPacksListing } from './bindings/AuthoringPacksListing';
export type { ServerEntry } from './bindings/ServerEntry';
export type { ServerListing } from './bindings/ServerListing';
export type { Featured } from './bindings/Featured';
export type { CacheInventory } from './bindings/CacheInventory';
export type { CacheInventoryEntry } from './bindings/CacheInventoryEntry';
export type { CacheUsageListing } from './bindings/CacheUsageListing';
export type { CacheUsageEntry } from './bindings/CacheUsageEntry';
export type { CacheUse } from './bindings/CacheUse';

// authoring config
export type { PackConfig } from './bindings/PackConfig';
export type { PackMeta } from './bindings/PackMeta';
export type { PackTier } from './bindings/PackTier';
export type { Visibility } from './bindings/Visibility';
export type { DeclaredMod } from './bindings/DeclaredMod';
export type { DeclaredAsset } from './bindings/DeclaredAsset';
export type { SourceDecl } from './bindings/SourceDecl';
export type { LoaderSpec } from './bindings/LoaderSpec';
export type { Display } from './bindings/Display';

// wire manifest (for the launcher-faithful preview)
export type { PackManifest } from './bindings/PackManifest';
export type { ModEntry } from './bindings/ModEntry';
export type { AssetEntry } from './bindings/AssetEntry';
export type { Source } from './bindings/Source';
export type { Requirement } from './bindings/Requirement';

// validate report (config vs SC archive)
export type { ValidateReport } from './bindings/ValidateReport';

// accounts (users + roles)
export type { UserRow } from './bindings/UserRow';
export type { UploadRow } from './bindings/UploadRow';

// registry browser (mods + builds, faceted)
export type { ModSummary } from './bindings/ModSummary';
export type { VersionRow } from './bindings/VersionRow';
export type { ReleaseRow } from './bindings/ReleaseRow';
export type { UnassignedJar } from './bindings/UnassignedJar';
export type { BuildSummary } from './bindings/BuildSummary';
export type { BuildModRow } from './bindings/BuildModRow';
export type { ModUse } from './bindings/ModUse';

// ── hand-written: operational + external (no Rust DTO) ──

export type JobStatus = 'running' | 'done' | 'failed';

export interface ModrinthHit {
  project_id: string;
  slug: string;
  title: string;
  description: string;
  icon_url?: string | null;
  author?: string;
}

export interface ModrinthVersion {
  id: string;
  project_id: string;
  version_number: string;
  // release channel: 'release' | 'beta' | 'alpha'
  version_type?: string;
  game_versions: string[];
  loaders: string[];
}

// GET /v1/jobs/:id -- `result` is present only for a finished dry-run.
export interface JobResult {
  job_id: string;
  kind: string;
  pack_id: string;
  status: JobStatus;
  log: string[];
  result?: DryRun | null;
}
