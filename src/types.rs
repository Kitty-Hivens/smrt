use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;

// ── Pack manifest ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackManifest {
    pub schema_version: u32,
    pub pack_id: String,
    pub pack_version: String,
    pub generated_at: String,
    pub minecraft: MinecraftSpec,
    pub loader: LoaderSpec,
    pub java: JavaSpec,
    pub mods: Vec<ModEntry>,
    #[serde(default)]
    pub assets: Vec<AssetEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftSpec {
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoaderSpec {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaSpec {
    pub major: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModEntry {
    pub filename: String,
    pub sha1: String,
    pub size_bytes: u64,
    #[serde(default = "default_true")]
    pub required: bool,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetEntry {
    pub dest: String,
    pub sha1: String,
    pub size_bytes: u64,
    #[serde(default = "default_true")]
    pub required: bool,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
}

/// Advisory display metadata for launcher UIs. Adding or removing this
/// block on an existing manifest is forward-compatible -- the wire schema
/// version stays at 2. Clients that don't recognise the block fall back
/// to defaults derived from `filename` / `dest`.
///
/// `icon_url`, `role`, and `requires` are additive launcher-side richer
/// UX hooks (per-item icons, role-grouped pickers, dependency graph
/// rendering). All three optional; manifests without them parse cleanly
/// on every client that reached the v2 schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Display {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incompatible_with: Vec<String>,
    /// SPDX license identifier where known (e.g. "MIT", "LGPL-3.0-only",
    /// "CC-BY-NC-SA-3.0"). Useful for a launcher to surface
    /// non-redistributable mods to the user. Absent for proprietary mods
    /// without an SPDX-compatible declaration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Source / project / wiki URL. Used by a launcher's "Learn more"
    /// affordance. Preferred order: mcmod.info url, Modrinth source_url,
    /// CurseForge project page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Per-item icon URL. Mirror serves directly for smrt_cache /
    /// smrt_static entries; Modrinth-sourced entries can leave this null
    /// and let the client resolve via the source's `project_id` against
    /// the Modrinth API. Null = client falls back to a letter avatar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// Short tag for grouping interchangeable mods. Launcher renders all
    /// mods with the same role as a single selectable slot ("Recipe
    /// viewer: JEI [v]" with REI / JER / EMI alternatives). Canonical
    /// values are mirror-curated; the launcher does not enumerate them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Same-manifest dependency declarations. Each entry's `filename`
    /// points at another mod in this pack's `mods[]`. Resolver
    /// validates the reference at install time; missing references
    /// surface as a warning rather than a hard failure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Requirement>,
}

/// Single edge in a mod's dependency DAG. [filename] must match a
/// mods[] entry's filename in the same manifest. [version_range]
/// follows Maven-style range syntax (`>=4.0`, `[1.0,2.0)`); null
/// means "any version present is acceptable". [optional] = true
/// means the consumer works without the dep but works better with
/// it -- launcher shows it greyed-out in the dep tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    pub filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_range: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Modrinth {
        project_id: String,
        version_id: String,
    },
    SmrtCache {
        url: String,
    },
    SmrtStatic {
        url: String,
    },
}

fn default_true() -> bool { true }

// ── Pack summary / listing ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct PackListing {
    pub schema_version: u32,
    pub generated_at: String,
    pub packs: Vec<PackSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestVersionsListing {
    pub schema_version: u32,
    pub pack_id: String,
    pub versions: Vec<String>,
}

// ── Server metadata ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEntry {
    pub schema_version: u32,
    pub server_id: String,
    pub pack_id: String,
    pub display_name: String,
    pub tagline: String,
    pub description_md: String,
    pub banner_url: String,
    #[serde(default)]
    pub gallery_urls: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discord_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    pub owner_display: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motd_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub founded_at: Option<String>,
    #[serde(default)]
    pub featured: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerListing {
    pub schema_version: u32,
    pub generated_at: String,
    pub servers: Vec<ServerEntry>,
}

// ── Featured ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Featured {
    pub schema_version: u32,
    pub generated_at: String,
    pub featured_servers: Vec<String>,
    pub featured_packs: Vec<String>,
}

// ── Cache inventory ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CacheInventory {
    pub schema_version: u32,
    pub generated_at: String,
    pub entries: Vec<CacheInventoryEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheInventoryEntry {
    pub sha1: String,
    pub size_bytes: u64,
}

// ── Health ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Health {
    pub schema_version: u32,
    pub status: &'static str,
    pub version: &'static str,
}

// ── Pack version comparison ────────────────────────────────────────────────

/// Numeric-tuple representation of a `YYYY.MM.DD[.N]` style version string.
/// Splits on `.` and parses each segment as `u64`; non-numeric segments
/// degrade to 0 so a malformed version still produces a comparable value
/// rather than panicking.
pub fn pack_version_tuple(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|seg| seg.parse::<u64>().unwrap_or(0))
        .collect()
}

/// Compare two pack versions per the spec rules: numeric tuple comparison
/// with missing trailing segments treated as `0`. So `2026.05.22` equals
/// `2026.05.22.0` and is strictly less than `2026.05.22.1`, and
/// `2026.05.22.10` sorts after `2026.05.22.2`. Both clients and the mirror
/// must use this comparison; plain `String` sort would order `.10` before
/// `.2` and breaks update detection.
pub fn compare_pack_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let mut at = pack_version_tuple(a);
    let mut bt = pack_version_tuple(b);
    let n = at.len().max(bt.len());
    at.resize(n, 0);
    bt.resize(n, 0);
    at.cmp(&bt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn compare_orders_two_digit_subversions_after_single_digit() {
        assert_eq!(compare_pack_versions("2026.05.22.2", "2026.05.22.10"), Ordering::Less);
    }

    #[test]
    fn compare_orders_dates_correctly() {
        assert_eq!(compare_pack_versions("2026.05.22", "2026.05.23"), Ordering::Less);
    }

    #[test]
    fn compare_treats_missing_trailing_segment_as_zero() {
        assert_eq!(compare_pack_versions("2026.05.22", "2026.05.22.0"), Ordering::Equal);
        assert_eq!(compare_pack_versions("2026.05.22", "2026.05.22.1"), Ordering::Less);
        assert_eq!(compare_pack_versions("2026.05.22.0.0", "2026.05.22"), Ordering::Equal);
    }

    #[test]
    fn mod_entry_serializes_without_display_block_when_absent() {
        let m = ModEntry {
            filename: "Quark.jar".into(),
            sha1: "abc".into(),
            size_bytes: 100,
            required: true,
            source: Source::SmrtCache { url: "u".into() },
            display: None,
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(!s.contains("display"),
            "absent display block must not serialize (forward-compat for old clients): {s}");
    }

    #[test]
    fn mod_entry_round_trips_display_block() {
        let json = r#"{
            "filename": "VoxelMap.jar",
            "sha1": "deadbeef",
            "size_bytes": 1024,
            "required": false,
            "source": {"type": "smrt_cache", "url": "https://example/v1/cache/de/deadbeef.jar"},
            "display": {
                "name": "VoxelMap",
                "description": "Minimap with waypoints",
                "category": "minimap",
                "incompatible_with": ["XaerosMinimap.jar", "JourneyMap.jar"]
            }
        }"#;
        let m: ModEntry = serde_json::from_str(json).unwrap();
        let d = m.display.expect("display deserialized");
        assert_eq!(d.name.as_deref(), Some("VoxelMap"));
        assert_eq!(d.category.as_deref(), Some("minimap"));
        assert_eq!(d.incompatible_with, vec!["XaerosMinimap.jar", "JourneyMap.jar"]);
    }

    #[test]
    fn mod_entry_round_trips_rich_display_block() {
        // icon_url + role + requires together. Wire format is what the
        // 2026-05-25 launcher spec extension expects; this test fails
        // loud if a field name drifts.
        let json = r#"{
            "filename": "appleskin.jar",
            "sha1": "abc",
            "size_bytes": 50000,
            "required": true,
            "source": {"type": "modrinth", "project_id": "EsAfCjCV", "version_id": "v"},
            "display": {
                "name": "AppleSkin",
                "category": "performance",
                "icon_url": "https://cdn.modrinth.com/data/EsAfCjCV/icon.png",
                "role": "info_overlay",
                "requires": [
                    {"filename": "Mixinbooter.jar", "version_range": ">=10.0", "optional": false}
                ]
            }
        }"#;
        let m: ModEntry = serde_json::from_str(json).unwrap();
        let d = m.display.expect("display deserialized");
        assert_eq!(d.icon_url.as_deref(), Some("https://cdn.modrinth.com/data/EsAfCjCV/icon.png"));
        assert_eq!(d.role.as_deref(), Some("info_overlay"));
        assert_eq!(d.requires.len(), 1);
        assert_eq!(d.requires[0].filename, "Mixinbooter.jar");
        assert_eq!(d.requires[0].version_range.as_deref(), Some(">=10.0"));
        assert!(!d.requires[0].optional);
    }

    #[test]
    fn requirement_optional_defaults_to_false_when_absent() {
        // version_range null AND optional missing -- a curator who
        // doesn't care about pinning a version should be able to write
        // `{"filename": "X.jar"}` and have it round-trip.
        let json = r#"{"filename": "AppliedEnergistics2.jar"}"#;
        let r: Requirement = serde_json::from_str(json).unwrap();
        assert_eq!(r.filename, "AppliedEnergistics2.jar");
        assert_eq!(r.version_range, None);
        assert!(!r.optional);
        // Round-trip preserves the omission shape.
        let s = serde_json::to_string(&r).unwrap();
        assert!(!s.contains("version_range"), "absent version_range must not serialize: {s}");
        assert!(s.contains("\"optional\":false"), "optional always serializes (no skip_if): {s}");
    }

    #[test]
    fn display_with_empty_requires_does_not_emit_field() {
        // Vec::is_empty skip is critical -- otherwise every existing
        // manifest entry would gain a noisy `"requires":[]` on next
        // build, churning every cache + breaking byte-equality
        // comparisons in client-side change detection.
        let d = Display {
            name: Some("X".into()),
            description: None, category: None,
            incompatible_with: vec![],
            license: None, url: None,
            icon_url: None, role: None,
            requires: vec![],
        };
        let s = serde_json::to_string(&d).unwrap();
        assert!(!s.contains("requires"), "empty requires must not serialize: {s}");
    }

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
        assert_eq!(s.icon_url.as_deref(),    Some("https://smrt.hivens.dev/v1/packs/Industrial/static/_nexira/icon.png"));
        assert_eq!(s.banner_url.as_deref(),  Some("https://smrt.hivens.dev/v1/packs/Industrial/static/_nexira/banner.png"));
        assert_eq!(s.gallery_urls.len(), 1);
        assert!(s.description_md.as_deref().unwrap().starts_with("# Industrial"));
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

    #[test]
    fn manifest_without_display_blocks_still_parses() {
        let json = r#"{
            "schema_version": 2,
            "pack_id": "Old",
            "pack_version": "2026.05.22",
            "generated_at": "2026-05-22T00:00:00Z",
            "minecraft": {"version": "1.12.2"},
            "loader": {"name": "forge", "version": "14.23.5.2922"},
            "java": {"major": 8},
            "mods": [{"filename": "X.jar", "sha1": "a", "size_bytes": 1, "required": true,
                "source": {"type": "smrt_cache", "url": "u"}}]
        }"#;
        let pm: PackManifest = serde_json::from_str(json).unwrap();
        assert!(pm.mods[0].display.is_none(),
            "manifests written before the display field landed must still parse");
    }
}
