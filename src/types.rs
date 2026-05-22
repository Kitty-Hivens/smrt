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
