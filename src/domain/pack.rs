//! Pack identity as the launcher sees it (`PackSummary`, listings) plus the
//! admin-authored `PackConfig` the build pipeline consumes. The config is a
//! distinct type from the wire manifest: authoring does not hand-write
//! `sha1` / `size_bytes` for Modrinth sources -- those are resolved at build.

use super::manifest::{Display, LoaderSpec};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ── Pack summary / listing ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
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
    pub icon_url: Option<String>,
    /// Wide hero image. Renders behind BrowsePackDetail hero text;
    /// falls back to the launcher's mirror gradient when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_url: Option<String>,
    /// Optional marketing screenshots. Rendered in a horizontal
    /// scroller on BrowsePackDetail when non-empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gallery_urls: Vec<String>,
    /// Long-form CommonMark description for the BrowsePackDetail
    /// About section. HTML is not parsed by the launcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_md: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct PackListing {
    pub schema_version: u32,
    pub generated_at: String,
    pub packs: Vec<PackSummary>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ManifestVersionsListing {
    pub schema_version: u32,
    pub pack_id: String,
    pub versions: Vec<String>,
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
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub featured: bool,
    pub mods: Vec<DeclaredMod>,
    #[serde(default)]
    pub assets: Vec<DeclaredAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct DeclaredMod {
    pub filename: String,
    #[serde(default = "default_true")]
    pub required: bool,
    /// Install-time default for an optional mod; the curator's default-off list
    /// flips it. Carried into the emitted ModEntry.
    #[serde(default = "default_true")]
    pub default_enabled: bool,
    pub source: SourceDecl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct DeclaredAsset {
    pub dest: String,
    #[serde(default = "default_true")]
    pub required: bool,
    pub source: SourceDecl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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
    }
}
