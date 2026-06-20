//! Jar metadata extraction + the build enrichment passes that mutate a
//! [`PackConfig`] in place. Each pass is a separate function so `smrt-pack`
//! can run them from its own subcommand and inspect the result between steps
//! -- e.g. fill name/description from mcmod.info, apply a role-table, then
//! infer requires.
//!
//! All passes are idempotent: re-running with the same inputs yields the same
//! output. Passes that fill optional fields prefer existing data over derived
//! data, so a manual override always wins against a heuristic source.

use super::archive::read_zip_entry;
use super::sources::cache_jar_path;
use crate::domain::PackConfig;
use crate::domain::SourceDecl;
use crate::domain::{Display, Requirement};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use tracing::{debug, info, warn};
use ts_rs::TS;

// ── mcmod.info ────────────────────────────────────────────────────────────

/// Subset of the 1.12.2-era Forge `mcmod.info` schema the build
/// pipeline reads. Real-world files are mostly the array form
/// `[{...mod...}, ...]`; some older mods wrap a single object in
/// `{"modListVersion": 2, "modList": [{...}]}`. [`read_mcmod_info`]
/// flattens both into `Vec<McModInfo>`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct McModInfo {
    #[serde(default)]
    pub modid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// 1.12-era Forge spells the author list `authorList`. Harvest reads it as a
    /// local, network-free author source (falling back to Modrinth only when the
    /// jar carries none).
    #[serde(default, rename = "authorList")]
    pub authors: Vec<String>,
    /// Path inside the jar to the mod's logo image (Forge `logoFile`), used to
    /// surface the mod's own icon in the panel.
    #[serde(default, rename = "logoFile")]
    pub logo_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
struct McModInfoListWrap {
    #[serde(rename = "modList")]
    mod_list: Vec<McModInfo>,
}

/// Reads `mcmod.info` from a jar's bytes. Returns the first entry if
/// any, since 1.12 mods overwhelmingly declare one modid per jar.
/// Returns `Ok(None)` for: jar without mcmod.info, malformed mcmod.info,
/// or empty mod list. Errors only on I/O / zip-corruption.
pub fn read_mcmod_info(jar_bytes: &[u8]) -> Result<Option<McModInfo>> {
    let mut zip = match zip::ZipArchive::new(Cursor::new(jar_bytes)) {
        Ok(z) => z,
        Err(e) => {
            debug!("jar is not a valid zip: {}", e);
            return Ok(None);
        }
    };
    let mut entry = match zip.by_name("mcmod.info") {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    let size = entry.size();
    let raw = read_zip_entry(&mut entry, size, "mcmod.info")?;

    // mcmod.info comes from many authors over many years. Lossy UTF-8
    // decode handles the occasional ISO-8859-1 file. BOM strip handles
    // the occasional UTF-8-BOM-prefixed file from Windows authors. .
    let decoded = String::from_utf8_lossy(&raw);
    let trimmed = decoded.trim_start_matches('\u{FEFF}').trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    // Two valid shapes per the Forge spec era:
    //   1. JSON array of mod entries
    //   2. JSON object with a `modList` field containing the array
    let parsed: Option<McModInfo> = if trimmed.starts_with('[') {
        serde_json::from_str::<Vec<McModInfo>>(trimmed)
            .ok()
            .and_then(|v| v.into_iter().next())
    } else {
        serde_json::from_str::<McModInfoListWrap>(trimmed)
            .ok()
            .and_then(|w| w.mod_list.into_iter().next())
    };

    Ok(parsed)
}

/// Extract a mod's embedded icon from its jar bytes: the Forge `mcmod.info`
/// `logoFile`, else a `fabric.mod.json` `icon`, else a conventional `pack.png` /
/// `icon.png` / `logo.png` at the jar root. Returns the image bytes plus a
/// content-type guessed from the entry name. `Ok(None)` when the jar has no
/// recognizable icon or isn't a readable zip.
pub fn jar_icon(jar_bytes: &[u8]) -> Result<Option<(Vec<u8>, &'static str)>> {
    let mut zip = match zip::ZipArchive::new(Cursor::new(jar_bytes)) {
        Ok(z) => z,
        Err(_) => return Ok(None),
    };

    let mut candidates: Vec<String> = Vec::new();
    if let Ok(Some(info)) = read_mcmod_info(jar_bytes) {
        let lf = info.logo_file.trim().trim_start_matches('/');
        if !lf.is_empty() {
            candidates.push(lf.to_string());
        }
    }
    if let Some(icon) = fabric_icon(jar_bytes) {
        candidates.push(icon);
    }
    for d in ["pack.png", "icon.png", "logo.png"] {
        candidates.push(d.to_string());
    }

    for name in candidates {
        let read = match zip.by_name(&name) {
            Ok(mut e) if e.is_file() => {
                let size = e.size();
                Some(read_zip_entry(&mut e, size, &name)?)
            }
            _ => None,
        };
        if let Some(bytes) = read
            && !bytes.is_empty()
        {
            return Ok(Some((bytes, content_type_for(&name))));
        }
    }
    Ok(None)
}

/// `fabric.mod.json` `icon` -- a string path, or a `{ "<size>": "path" }` map
/// from which any entry serves.
fn fabric_icon(jar_bytes: &[u8]) -> Option<String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(jar_bytes)).ok()?;
    let mut entry = zip.by_name("fabric.mod.json").ok()?;
    let size = entry.size();
    let raw = read_zip_entry(&mut entry, size, "fabric.mod.json").ok()?;
    let v: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let path = match v.get("icon")? {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(m) => m.values().find_map(|x| x.as_str())?.to_string(),
        _ => return None,
    };
    Some(path.trim_start_matches('/').to_string())
}

fn content_type_for(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".jpg") || n.ends_with(".jpeg") {
        "image/jpeg"
    } else if n.ends_with(".gif") {
        "image/gif"
    } else {
        "image/png"
    }
}

// ── Pass 1: enrich display from mcmod.info ────────────────────────────────

#[derive(Debug, Default)]
pub struct McModEnrichReport {
    pub mods_with_info: u32,
    pub mods_filled: u32,
    pub mods_skipped_modrinth: u32,
    pub mods_skipped_no_info: u32,
    pub mods_skipped_already_complete: u32,
}

/// Fills `display.name / description / url` on every smrt_cache-sourced
/// mod whose jar has a parseable `mcmod.info`. Existing values win --
/// this pass never overwrites a field the human already filled in.
///
/// Modrinth-sourced mods are skipped here; their display metadata
/// comes from the Modrinth project API and lands via a separate pass
/// (not yet implemented in this module).
pub fn enrich_from_mcmod_info(
    config: &mut PackConfig,
    storage: &Path,
) -> Result<McModEnrichReport> {
    let mut report = McModEnrichReport::default();
    for m in config.mods.iter_mut() {
        let sha1 = match &m.source {
            SourceDecl::SmrtCache { sha1 } => sha1.clone(),
            SourceDecl::Modrinth { .. } => {
                report.mods_skipped_modrinth += 1;
                continue;
            }
            SourceDecl::SmrtStatic { .. } => continue,
        };

        let display = m.display.get_or_insert_with(default_display);
        if display.name.is_some() && display.description.is_some() && display.url.is_some() {
            report.mods_skipped_already_complete += 1;
            continue;
        }

        let jar_path = cache_jar_path(storage, &sha1)?;
        let bytes = match fs::read(&jar_path) {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    "cache jar {} not readable for {}: {}",
                    jar_path.display(),
                    m.filename,
                    e
                );
                continue;
            }
        };
        let info = match read_mcmod_info(&bytes)? {
            Some(i) => i,
            None => {
                report.mods_skipped_no_info += 1;
                continue;
            }
        };
        report.mods_with_info += 1;

        let mut filled_anything = false;
        if display.name.is_none() && !info.name.trim().is_empty() {
            display.name = Some(info.name.trim().to_string());
            filled_anything = true;
        }
        if display.description.is_none() && !info.description.trim().is_empty() {
            display.description = Some(info.description.trim().to_string());
            filled_anything = true;
        }
        if display.url.is_none() && !info.url.trim().is_empty() {
            display.url = Some(info.url.trim().to_string());
            filled_anything = true;
        }
        if filled_anything {
            report.mods_filled += 1;
        }
    }
    info!(
        with_info = report.mods_with_info,
        filled = report.mods_filled,
        skipped_mr = report.mods_skipped_modrinth,
        skipped_noinf = report.mods_skipped_no_info,
        skipped_full = report.mods_skipped_already_complete,
        "enrich-from-mcmod-info complete"
    );
    Ok(report)
}

// ── Pass 2: apply role-table ──────────────────────────────────────────────

/// Authored mapping of mod filename to role string.
/// Loaded from a TOML file via [`load_role_table`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct RoleTable {
    #[serde(default)]
    pub roles: HashMap<String, String>,
}

pub fn load_role_table(path: &Path) -> Result<RoleTable> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading role table {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parsing role table {}", path.display()))
}

#[derive(Debug, Default)]
pub struct RoleApplyReport {
    pub applied: u32,
    pub skipped_already_set: u32,
    pub unmatched_in_table: Vec<String>,
}

/// Writes `display.role` on every mod whose filename matches an entry
/// in the role-table. Existing `display.role` wins -- the table never
/// overrides a manually-set value. Returns the list of table entries
/// that did not match any mod so typos can be spotted.
pub fn apply_role_table(config: &mut PackConfig, table: &RoleTable) -> Result<RoleApplyReport> {
    let mut report = RoleApplyReport::default();
    let filenames: std::collections::HashSet<&str> =
        config.mods.iter().map(|m| m.filename.as_str()).collect();

    for fname in table.roles.keys() {
        if !filenames.contains(fname.as_str()) {
            report.unmatched_in_table.push(fname.clone());
        }
    }
    report.unmatched_in_table.sort();

    for m in config.mods.iter_mut() {
        let Some(role) = table.roles.get(&m.filename) else {
            continue;
        };
        let display = m.display.get_or_insert_with(default_display);
        if display.role.is_some() {
            report.skipped_already_set += 1;
            continue;
        }
        display.role = Some(role.clone());
        report.applied += 1;
    }
    info!(
        applied = report.applied,
        skipped = report.skipped_already_set,
        unmatched = report.unmatched_in_table.len(),
        "apply-role-table complete"
    );
    Ok(report)
}

// ── Pass 3: infer requires from mcmod.info dependencies ───────────────────

#[derive(Debug, Default)]
pub struct InferRequiresReport {
    pub mods_with_deps: u32,
    pub edges_added: u32,
    pub edges_skipped_unresolved: Vec<(String, String)>,
}

/// Walks every smrt_cache-sourced mod's `mcmod.info.dependencies` list
/// and emits `display.requires` entries pointing at sibling mods in the
/// same pack. Modid -> filename resolution uses the modid declared
/// inside each jar's own mcmod.info, so this only works for jars that
/// declare one. Modrinth-sourced mods are skipped (their deps live in
/// the Modrinth API and need a separate pass).
///
/// Existing `display.requires` wins -- this pass never replaces an
/// authored list, only fills an empty one.
pub fn infer_requires_from_mcmod_info(
    config: &mut PackConfig,
    storage: &Path,
) -> Result<InferRequiresReport> {
    // First pass: build modid -> filename map across the pack.
    let mut modid_to_filename: HashMap<String, String> = HashMap::new();
    for m in &config.mods {
        let sha1 = match &m.source {
            SourceDecl::SmrtCache { sha1 } => sha1.clone(),
            _ => continue,
        };
        let jar_path = cache_jar_path(storage, &sha1)?;
        let Ok(bytes) = fs::read(&jar_path) else {
            continue;
        };
        let Some(info) = read_mcmod_info(&bytes)? else {
            continue;
        };
        if info.modid.is_empty() {
            continue;
        }
        // First-write wins so a duplicate modid (e.g. shadowed jar) is
        // logged but does not silently overwrite the canonical mapping.
        if let Some(existing) = modid_to_filename.get(&info.modid) {
            warn!(
                "duplicate modid {} mapped to both {} and {}; keeping the first",
                info.modid, existing, m.filename
            );
            continue;
        }
        modid_to_filename.insert(info.modid.clone(), m.filename.clone());
    }
    debug!(
        modids = modid_to_filename.len(),
        "built modid->filename map"
    );

    // Second pass: emit display.requires for each mod whose mcmod.info
    // declares dependencies that resolve against the map.
    let mut report = InferRequiresReport::default();
    let modids = modid_to_filename.clone();
    for m in config.mods.iter_mut() {
        let sha1 = match &m.source {
            SourceDecl::SmrtCache { sha1 } => sha1.clone(),
            _ => continue,
        };
        if let Some(d) = &m.display
            && !d.requires.is_empty()
        {
            continue;
        }
        let jar_path = cache_jar_path(storage, &sha1)?;
        let Ok(bytes) = fs::read(&jar_path) else {
            continue;
        };
        let Some(info) = read_mcmod_info(&bytes)? else {
            continue;
        };
        if info.dependencies.is_empty() {
            continue;
        }
        report.mods_with_deps += 1;

        let mut edges = Vec::new();
        for dep_modid in &info.dependencies {
            // mcmod.info dependencies are bare modids in 1.12 era.
            // Forge's @Mod annotation has more structured deps with
            // version ranges; that's a future enrichment path.
            match modids.get(dep_modid) {
                Some(target_fname) => edges.push(Requirement {
                    filename: target_fname.clone(),
                    version_range: None,
                    optional: false,
                }),
                None => report
                    .edges_skipped_unresolved
                    .push((m.filename.clone(), dep_modid.clone())),
            }
        }
        if !edges.is_empty() {
            report.edges_added += edges.len() as u32;
            let display = m.display.get_or_insert_with(default_display);
            display.requires = edges;
        }
    }
    info!(
        with_deps = report.mods_with_deps,
        edges = report.edges_added,
        unresolved = report.edges_skipped_unresolved.len(),
        "infer-requires-from-mcmod-info complete"
    );
    Ok(report)
}

// ── helpers ───────────────────────────────────────────────────────────────

fn default_display() -> Display {
    Display {
        name: None,
        description: None,
        category: None,
        incompatible_with: Vec::new(),
        license: None,
        url: None,
        icon_url: None,
        role: None,
        requires: Vec::new(),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LoaderSpec;
    use crate::domain::{DeclaredMod, PackConfig, SourceDecl};
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;

    fn empty_config() -> PackConfig {
        PackConfig {
            pack_id: "Test".into(),
            display_name: "Test".into(),
            tagline: String::new(),
            minecraft_version: "1.12.2".into(),
            loader: LoaderSpec {
                name: "forge".into(),
                version: "14.23.5.2922".into(),
            },
            java_major: 8,
            version: None,
            tags: Vec::new(),
            featured: false,
            mods: Vec::new(),
            assets: Vec::new(),
            pack_meta: Default::default(),
        }
    }

    fn write_test_jar(dir: &Path, sha1: &str, mcmod_json: &str) -> Result<()> {
        // Place the fixture where the code reads it -- via the same layout helper.
        let jar_path = cache_jar_path(dir, sha1)?;
        if let Some(parent) = jar_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let f = fs::File::create(&jar_path)?;
        let mut zw = zip::ZipWriter::new(f);
        zw.start_file("mcmod.info", SimpleFileOptions::default())?;
        zw.write_all(mcmod_json.as_bytes())?;
        zw.finish()?;
        Ok(())
    }

    #[test]
    fn read_mcmod_info_handles_array_form() {
        // Standard form from 99% of 1.12.2 mods.
        let bytes = build_jar_bytes(
            r#"[{"modid":"appleskin","name":"AppleSkin","description":"Hunger viz","url":"https://modrinth.com/mod/appleskin","dependencies":["appleskin-api"]}]"#,
        );
        let info = read_mcmod_info(&bytes).unwrap().unwrap();
        assert_eq!(info.modid, "appleskin");
        assert_eq!(info.name, "AppleSkin");
        assert_eq!(info.dependencies, vec!["appleskin-api"]);
    }

    #[test]
    fn read_mcmod_info_handles_object_wrap_form() {
        // Older "modListVersion": 2 schema.
        let bytes = build_jar_bytes(
            r#"{"modListVersion":2,"modList":[{"modid":"oldmod","name":"OldMod"}]}"#,
        );
        let info = read_mcmod_info(&bytes).unwrap().unwrap();
        assert_eq!(info.modid, "oldmod");
    }

    #[test]
    fn read_mcmod_info_returns_none_when_absent() {
        let bytes = build_jar_bytes_without_mcmod();
        let info = read_mcmod_info(&bytes).unwrap();
        assert!(info.is_none());
    }

    #[test]
    fn read_mcmod_info_tolerates_bom_and_whitespace() {
        let blob = "\u{FEFF}  [{\"modid\":\"bom_mod\"}]  ".to_string();
        let bytes = build_jar_bytes(&blob);
        let info = read_mcmod_info(&bytes).unwrap().unwrap();
        assert_eq!(info.modid, "bom_mod");
    }

    #[test]
    fn enrich_fills_only_missing_fields() {
        let dir = TempDir::new().unwrap();
        let sha = "a".repeat(40);
        write_test_jar(
            dir.path(),
            &sha,
            r#"[{"modid":"x","name":"FromJar","description":"FromJarDesc","url":"https://fromjar"}]"#,
        ).unwrap();

        let mut cfg = empty_config();
        cfg.mods.push(DeclaredMod {
            filename: "X.jar".into(),
            required: true,
            default_enabled: true,
            source: SourceDecl::SmrtCache { sha1: sha.clone() },
            display: Some(Display {
                name: Some("CuratorName".into()), // existing -- must win
                description: None,
                category: None,
                incompatible_with: Vec::new(),
                license: None,
                url: None,
                icon_url: None,
                role: None,
                requires: Vec::new(),
            }),
            note: None,
        });

        let report = enrich_from_mcmod_info(&mut cfg, dir.path()).unwrap();
        assert_eq!(report.mods_with_info, 1);
        let d = cfg.mods[0].display.as_ref().unwrap();
        assert_eq!(d.name.as_deref(), Some("CuratorName"), "curator wins");
        assert_eq!(d.description.as_deref(), Some("FromJarDesc"));
        assert_eq!(d.url.as_deref(), Some("https://fromjar"));
    }

    #[test]
    fn role_table_applies_and_reports_unmatched() {
        let mut cfg = empty_config();
        for fname in ["JEI.jar", "Xaero.jar", "AlreadyHasRole.jar"] {
            cfg.mods.push(DeclaredMod {
                filename: fname.into(),
                required: true,
                default_enabled: true,
                source: SourceDecl::SmrtCache {
                    sha1: "a".repeat(40),
                },
                display: if fname == "AlreadyHasRole.jar" {
                    Some(Display {
                        role: Some("custom".into()),
                        name: None,
                        description: None,
                        category: None,
                        incompatible_with: Vec::new(),
                        license: None,
                        url: None,
                        icon_url: None,
                        requires: Vec::new(),
                    })
                } else {
                    None
                },
                note: None,
            });
        }
        let mut table = RoleTable::default();
        table.roles.insert("JEI.jar".into(), "recipe_viewer".into());
        table.roles.insert("Xaero.jar".into(), "minimap".into());
        table
            .roles
            .insert("AlreadyHasRole.jar".into(), "overridden".into());
        table.roles.insert("Typo.jar".into(), "ignored".into());

        let r = apply_role_table(&mut cfg, &table).unwrap();
        assert_eq!(r.applied, 2);
        assert_eq!(r.skipped_already_set, 1);
        assert_eq!(r.unmatched_in_table, vec!["Typo.jar".to_string()]);
        assert_eq!(
            cfg.mods[0].display.as_ref().unwrap().role.as_deref(),
            Some("recipe_viewer")
        );
        assert_eq!(
            cfg.mods[1].display.as_ref().unwrap().role.as_deref(),
            Some("minimap")
        );
        assert_eq!(
            cfg.mods[2].display.as_ref().unwrap().role.as_deref(),
            Some("custom")
        );
    }

    #[test]
    fn infer_requires_resolves_modid_dependencies() {
        let dir = TempDir::new().unwrap();
        let sha_jei = "1".repeat(40);
        let sha_addon = "2".repeat(40);
        write_test_jar(dir.path(), &sha_jei, r#"[{"modid":"jei","name":"JEI"}]"#).unwrap();
        write_test_jar(
            dir.path(),
            &sha_addon,
            r#"[{"modid":"jeiaddon","name":"JEI Addon","dependencies":["jei"]}]"#,
        )
        .unwrap();

        let mut cfg = empty_config();
        cfg.mods.push(DeclaredMod {
            filename: "JEI.jar".into(),
            required: true,
            default_enabled: true,
            source: SourceDecl::SmrtCache { sha1: sha_jei },
            display: None,
            note: None,
        });
        cfg.mods.push(DeclaredMod {
            filename: "JEIAddon.jar".into(),
            required: true,
            default_enabled: true,
            source: SourceDecl::SmrtCache { sha1: sha_addon },
            display: None,
            note: None,
        });

        let report = infer_requires_from_mcmod_info(&mut cfg, dir.path()).unwrap();
        assert_eq!(report.edges_added, 1);
        let addon_deps = &cfg.mods[1].display.as_ref().unwrap().requires;
        assert_eq!(addon_deps.len(), 1);
        assert_eq!(addon_deps[0].filename, "JEI.jar");
    }

    #[test]
    fn infer_requires_reports_unresolved_modids() {
        let dir = TempDir::new().unwrap();
        let sha = "3".repeat(40);
        write_test_jar(
            dir.path(),
            &sha,
            r#"[{"modid":"lonely","dependencies":["ghost"]}]"#,
        )
        .unwrap();

        let mut cfg = empty_config();
        cfg.mods.push(DeclaredMod {
            filename: "Lonely.jar".into(),
            required: true,
            default_enabled: true,
            source: SourceDecl::SmrtCache { sha1: sha },
            display: None,
            note: None,
        });

        let report = infer_requires_from_mcmod_info(&mut cfg, dir.path()).unwrap();
        assert_eq!(report.edges_added, 0);
        assert_eq!(
            report.edges_skipped_unresolved,
            vec![("Lonely.jar".into(), "ghost".into())]
        );
    }

    fn build_jar_bytes(mcmod_json: &str) -> Vec<u8> {
        let buf = Cursor::new(Vec::new());
        let mut zw = zip::ZipWriter::new(buf);
        zw.start_file("mcmod.info", SimpleFileOptions::default())
            .unwrap();
        zw.write_all(mcmod_json.as_bytes()).unwrap();
        zw.finish().unwrap().into_inner()
    }

    fn build_jar_bytes_without_mcmod() -> Vec<u8> {
        let buf = Cursor::new(Vec::new());
        let mut zw = zip::ZipWriter::new(buf);
        zw.start_file("META-INF/MANIFEST.MF", SimpleFileOptions::default())
            .unwrap();
        zw.write_all(b"Manifest-Version: 1.0\n").unwrap();
        zw.finish().unwrap().into_inner()
    }

    #[test]
    fn jar_icon_prefers_logofile_then_pack_png_else_none() {
        // mcmod.info logoFile wins over a present pack.png
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zw.start_file("mcmod.info", SimpleFileOptions::default())
            .unwrap();
        zw.write_all(br#"[{"modid":"x","logoFile":"assets/x/logo.png"}]"#)
            .unwrap();
        zw.start_file("assets/x/logo.png", SimpleFileOptions::default())
            .unwrap();
        zw.write_all(b"LOGO").unwrap();
        zw.start_file("pack.png", SimpleFileOptions::default())
            .unwrap();
        zw.write_all(b"PACK").unwrap();
        let jar = zw.finish().unwrap().into_inner();
        let (bytes, ct) = jar_icon(&jar).unwrap().unwrap();
        assert_eq!(bytes, b"LOGO", "logoFile takes priority");
        assert_eq!(ct, "image/png");

        // a manifest-only jar yields no icon
        assert!(
            jar_icon(&build_jar_bytes_without_mcmod())
                .unwrap()
                .is_none()
        );

        // pack.png is the fallback when there's no logoFile
        let mut zw3 = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zw3.start_file("pack.png", SimpleFileOptions::default())
            .unwrap();
        zw3.write_all(b"P").unwrap();
        let jar3 = zw3.finish().unwrap().into_inner();
        assert_eq!(jar_icon(&jar3).unwrap().unwrap().0, b"P");
    }
}
