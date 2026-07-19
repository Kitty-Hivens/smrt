//! Pack identity as the launcher sees it (`PackSummary`, listings) plus the
//! admin-authored `PackConfig` the build pipeline consumes. The config is a
//! distinct type from the wire manifest: authoring does not hand-write
//! `sha1` / `size_bytes` for Modrinth sources -- those are resolved at build.

use super::manifest::{Display, LoaderSpec};
use super::version::VersionChannel;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

// ── Ownership ──────────────────────────────────────────────────────────────

/// The deployment operator's GitHub uid, from `SMRT_OPERATOR_UID`, read once.
/// Packs authored before ownership existed backfill their `owner` to it, and
/// operator-authored packs default to it. 0 (no operator identity) when the
/// variable is unset -- a fresh self-hosted instance works before one is
/// configured. Lives here rather than `Config` because serde `default` fns
/// take no arguments; the one env read is cached for the process lifetime.
fn operator_uid() -> i64 {
    static UID: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *UID.get_or_init(|| {
        std::env::var("SMRT_OPERATOR_UID")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    })
}

/// Curation tier. `official` = the mirror's own packs (the launcher's catalog,
/// no personal byline); `community` = a member's pack. The launcher reads
/// official only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export, export_to = "bindings/")]
#[serde(rename_all = "snake_case")]
pub enum PackTier {
    Official,
    Community,
}

/// Publication state. Only `published` packs reach the public listing; `draft`
/// is work-in-progress, `unlisted` is reachable by direct id but off the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export, export_to = "bindings/")]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Draft,
    Unlisted,
    Published,
}

/// Backfill defaults, chosen so an existing config/summary with none of these
/// fields reads as an owned, official, published pack (which is what every pack
/// predating ownership is). `pub(crate)` so authoring code that mints a fresh
/// operator pack reuses the same defaults instead of re-hardcoding them.
pub(crate) fn default_owner() -> i64 {
    operator_uid()
}
pub(crate) fn default_tier() -> PackTier {
    PackTier::Official
}
pub(crate) fn default_visibility() -> Visibility {
    Visibility::Published
}

// ── Pack summary / listing ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS, ToSchema)]
#[ts(export, export_to = "bindings/")]
pub struct PackSummary {
    pub pack_id: String,
    pub display_name: String,
    pub tagline: String,
    pub minecraft_version: String,
    pub latest_pack_version: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub featured: bool,
    /// Square pack icon. Renders in BrowsePackCard avatar slot +
    /// BrowsePackDetail hero on the launcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub icon_url: Option<String>,
    /// Wide hero image. Renders behind BrowsePackDetail hero text;
    /// falls back to the launcher's mirror gradient when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub banner_url: Option<String>,
    /// Optional marketing screenshots. Rendered in a horizontal
    /// scroller on BrowsePackDetail when non-empty.
    #[serde(default)]
    pub gallery_urls: Vec<String>,
    /// Long-form CommonMark description for the BrowsePackDetail
    /// About section. HTML is not parsed by the launcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description_md: Option<String>,
    /// GitHub uid of the pack owner. Official packs are owned by the operator;
    /// community packs by their member. Server-controlled -- set at authoring.
    #[serde(default = "default_owner")]
    #[ts(type = "number")]
    pub owner: i64,
    #[serde(default = "default_tier")]
    pub tier: PackTier,
    #[serde(default = "default_visibility")]
    pub visibility: Visibility,
    /// Source pack id when this pack is a fork; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fork_of: Option<String>,
    /// When the latest build was produced (the latest manifest's
    /// `generated_at`, RFC 3339). Derived at read time from the manifest --
    /// never persisted in `summary.json`; absent when the pack has no build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub latest_built_at: Option<String>,
    /// Channel of `latest_pack_version`; same derivation and absence rules
    /// as `latest_built_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub latest_channel: Option<VersionChannel>,
}

#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export, export_to = "bindings/")]
pub struct PackListing {
    pub schema_version: u32,
    pub generated_at: String,
    pub packs: Vec<PackSummary>,
}

/// A published community pack for the public Community listing: the pack summary
/// plus the owner's GitHub login (resolved from the uid) for the `by <user>`
/// byline. Community packs are browseable on the site but never in the launcher's
/// official `/v1/packs` catalog.
#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export, export_to = "bindings/")]
pub struct CommunityPack {
    pub summary: PackSummary,
    pub owner_login: String,
}

/// One retained build of a pack, as the version picker sees it: the version
/// label plus the metadata a launcher needs to render and order a list of
/// builds without fetching each manifest. Field names follow the Modrinth
/// version object (`version_number` / `version_type` / `date_published`).
#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export, export_to = "bindings/")]
pub struct ManifestBuildInfo {
    pub version_number: String,
    /// The manifest's stored channel; for manifests predating the field,
    /// recovered from the legacy string rule (`SNAPSHOT-` prefix = beta).
    pub version_type: VersionChannel,
    /// The manifest's `generated_at` (RFC 3339): when the build produced it.
    pub date_published: String,
    /// The manifest's content fingerprint where present -- the reliable
    /// "did the content change?" signal between two builds.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fingerprint: Option<String>,
    /// Curator-authored release notes, where the build carries them.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub changelog: Option<String>,
    #[ts(type = "number")]
    pub mods_count: u64,
    #[ts(type = "number")]
    pub assets_count: u64,
}

#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export, export_to = "bindings/")]
pub struct ManifestVersionsListing {
    pub schema_version: u32,
    pub pack_id: String,
    /// The version `manifests/latest` currently points at; absent when the
    /// pack has no published build.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub latest: Option<String>,
    /// Per-build metadata, newest first (ordered by `date_published`).
    pub builds: Vec<ManifestBuildInfo>,
}

/// Pack ids that carry editable authoring inputs (a config.json under
/// `packs/<id>/authoring/`), including packs not yet built. Admin-only:
/// authoring inputs are not part of the public read API.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct AuthoringPacksListing {
    pub schema_version: u32,
    pub packs: Vec<String>,
}

// ── Pack config (admin-authored authoring input) ───────────────────────────

/// Admin-authored declaration of a pack. The build step turns this into a wire
/// `PackManifest` by resolving each source against Modrinth or the local
/// storage tree. Distinct from `PackManifest` because authoring does not
/// require admin to hand-write `sha1` and `size_bytes` for Modrinth sources --
/// those are looked up at build time.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct PackConfig {
    pub pack_id: String,
    pub display_name: String,
    pub tagline: String,
    pub minecraft_version: String,
    pub loader: LoaderSpec,
    pub java_major: u32,
    /// Human semver-ish line for the build version string
    /// (`SNAPSHOT-<version>-<date>`). Pre-1.0 packs sit at `0.0.x`; the operator
    /// bumps it rarely. Absent -> `0.0.0`. The date + same-day counter advance
    /// automatically, so this is the only version part anyone hand-edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub featured: bool,
    pub mods: Vec<DeclaredMod>,
    #[serde(default)]
    pub assets: Vec<DeclaredAsset>,
    #[serde(default)]
    pub pack_meta: PackMeta,
    /// GitHub uid of the pack owner. Official packs are owned by the operator;
    /// community packs by their member. Server-controlled -- never taken from a
    /// client config edit (see `put_pack_config`).
    #[serde(default = "default_owner")]
    #[ts(type = "number")]
    pub owner: i64,
    #[serde(default = "default_tier")]
    pub tier: PackTier,
    #[serde(default = "default_visibility")]
    pub visibility: Visibility,
    /// Source pack id when this pack is a fork; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fork_of: Option<String>,
}

/// Pack-card metadata (icon / banner / gallery / long description) merged into
/// the emitted `summary.json` at build time. Every field optional; absent fields
/// stay out of summary.json (per the `skip_serializing_if` on PackSummary).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct PackMeta {
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub banner_url: Option<String>,
    #[serde(default)]
    pub gallery_urls: Vec<String>,
    #[serde(default)]
    pub description_md: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct DeclaredMod {
    pub filename: String,
    /// Install-time default: whether the mod ships enabled. Every mod is toggleable
    /// -- there is no hand-set "required" flag. A mod another present mod hard-depends
    /// on is marked required on the emitted ModEntry automatically at build time.
    #[serde(default = "default_true")]
    pub default_enabled: bool,
    pub source: SourceDecl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub display: Option<Display>,
    /// Curator-assigned stable identity, carried into the emitted ModEntry so the
    /// launcher can key an optional mod's toggle by it across version bumps (ADR
    /// 0002). Optional; a Modrinth mod already has a stable key in its project id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub slug: Option<String>,
    /// True when depfill appended this entry as a resolved hard dependency
    /// (server-managed), false for curator-declared mods. A pulled entry is
    /// sticky: a save whose body lacks it merges it back in (a client that
    /// never saw it must not delete it), and it is dropped only when no
    /// curator-declared mod still reaches it through hard requires edges.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pulled: bool,
}

/// `skip_serializing_if` helper: omit `pulled` when false so existing configs
/// and the wire stay byte-identical for curator-declared mods.
fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct DeclaredAsset {
    pub dest: String,
    #[serde(default = "default_true")]
    pub required: bool,
    pub source: SourceDecl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub display: Option<Display>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceDecl {
    Modrinth {
        project_id: String,
        version_id: String,
    },
    SmrtCache {
        sha1: String,
    },
    SmrtStatic {
        rel_path: String,
    },
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_summary_round_trips_rich_metadata() {
        // r##"..."## (two hashes) -- the description_md contains "# ",
        // and r#"..."# would terminate at the first `"#` it hit. Two
        // hashes leave room for a single hash inside.
        let json = r##"{
            "pack_id": "Industrial",
            "display_name": "Industrial",
            "tagline": "Heavy industry and automation.",
            "minecraft_version": "1.12.2",
            "latest_pack_version": "2026.05.23.1",
            "tags": ["tech", "industrial"],
            "featured": true,
            "icon_url": "https://smrt.hivens.dev/v1/packs/Industrial/static/_nexira/icon.png",
            "banner_url": "https://smrt.hivens.dev/v1/packs/Industrial/static/_nexira/banner.png",
            "gallery_urls": [
                "https://smrt.hivens.dev/v1/packs/Industrial/static/_nexira/g1.png"
            ],
            "description_md": "# Industrial\n\nLong-form copy."
        }"##;
        let s: PackSummary = serde_json::from_str(json).unwrap();
        assert_eq!(
            s.icon_url.as_deref(),
            Some("https://smrt.hivens.dev/v1/packs/Industrial/static/_nexira/icon.png")
        );
        assert_eq!(
            s.banner_url.as_deref(),
            Some("https://smrt.hivens.dev/v1/packs/Industrial/static/_nexira/banner.png")
        );
        assert_eq!(s.gallery_urls.len(), 1);
        assert!(
            s.description_md
                .as_deref()
                .unwrap()
                .starts_with("# Industrial")
        );
    }

    #[test]
    fn pack_summary_without_rich_metadata_parses() {
        // Existing summary.json files written before the rich-metadata
        // extension must still parse.
        let json = r#"{
            "pack_id": "Bare",
            "display_name": "Bare",
            "tagline": "",
            "minecraft_version": "1.12.2",
            "latest_pack_version": "2026.06.01",
            "tags": []
        }"#;
        let s: PackSummary = serde_json::from_str(json).unwrap();
        assert!(s.icon_url.is_none());
        assert!(s.banner_url.is_none());
        assert!(s.gallery_urls.is_empty());
        assert!(s.description_md.is_none());
        // the ownership fields backfill via serde defaults, so a summary predating
        // them reads as an owned, official, published pack -- no migration needed
        assert_eq!(s.owner, default_owner());
        assert_eq!(s.tier, PackTier::Official);
        assert_eq!(s.visibility, Visibility::Published);
        assert!(s.fork_of.is_none());
    }
}
