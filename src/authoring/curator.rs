//! Enrichment passes that mutate a [`PackConfig`] in place. Each pass is
//! a separate function so the curator can run them in any order via the
//! corresponding `smrt-pack` subcommand and inspect the result between
//! steps -- e.g. fill name/description from mcmod.info, then apply
//! role-table, then infer requires.
//!
//! All passes are idempotent: re-running with the same inputs yields the
//! same output. Passes that fill optional fields prefer existing curator
//! data over derived data, so a manual role-table override always wins
//! against a heuristic source.

use crate::domain::{PackConfig, SourceDecl};
use crate::domain::{Display, Requirement};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

// ── mcmod.info ────────────────────────────────────────────────────────────

/// Subset of the 1.12.2-era Forge `mcmod.info` schema the curator
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
    let mut raw = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut raw)
        .context("reading mcmod.info from zip")?;

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
/// mod whose jar has a parseable `mcmod.info`. Existing curator-written
/// values win -- this pass never overwrites a field the human already
/// filled in.
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

/// Curator-authored mapping of mod filename to role string.
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

/// Pack-level rich metadata authored by the curator in a TOML file.
/// Merged into the emitted `summary.json` by `smrt-pack build` when
/// passed via `--pack-meta`. Every field optional; absent fields stay
/// out of summary.json (per the `skip_serializing_if` on PackSummary).
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

pub fn load_pack_meta(path: &Path) -> Result<PackMeta> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading pack meta {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parsing pack meta {}", path.display()))
}

// ── Omnibus curator config ────────────────────────────────────────────────

/// All-in-one curator file. Single TOML that drives a deterministic
/// chain of mutations on a [`PackConfig`] via [`apply_curator`].
///
/// Daily workflow for a SC-derived pack:
///   1. `smrt-pack bootstrap` (SC zip -> starter PackConfig)
///   2. `smrt-pack apply-curator --curator curator.toml`
///   3. `smrt-pack build --curator curator.toml`
///
/// The individual passes ([`enrich_from_mcmod_info`],
/// [`apply_role_table`], [`infer_requires_from_mcmod_info`]) remain
/// callable from their own subcommands for power-user / debugging
/// scenarios, but the canonical pipeline goes through this one file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct Curator {
    #[serde(default)]
    pub pack_meta: PackMeta,
    #[serde(default)]
    pub mark_optional: MarkOptional,
    /// `filename -> [incompatible filenames]`, written to each mod's
    /// `display.incompatible_with`. Mutual at the launcher, so only one side
    /// needs declaring (FoamFix.jar = ["!mixinbooter-10.7.jar"]).
    #[serde(default)]
    pub incompatible: HashMap<String, Vec<String>>,
    /// Optional mod filenames that should install DISABLED by default
    /// (`default_enabled = false`); the user opts in. Use for an optional that
    /// conflicts with a default-on mod (FoamFix vs Mixinbooter).
    #[serde(default)]
    pub default_off: Vec<String>,
    #[serde(default)]
    pub substitute: HashMap<String, SubstituteEntry>,
    #[serde(default)]
    pub role_table: HashMap<String, String>,
    #[serde(default)]
    pub category_table: HashMap<String, String>,
    #[serde(default)]
    pub extra_mods: Vec<ExtraMod>,
    #[serde(default)]
    pub extra_assets: Vec<ExtraAsset>,
    #[serde(default)]
    pub drop_assets: DropAssets,
    #[serde(default)]
    pub generate: GenerateConfig,
}

/// Asset destinations the curator wants stripped from the
/// emitted manifest. Use case: SC's archive ships ~80 mod-default
/// config files (foamfix.cfg, chisel.cfg, AE2's items.csv dump,
/// stale CoFH world JSONs, etc) that every Forge mod regenerates
/// from its own jar resources on first launch. Shipping the
/// defaults pre-baked locks every install into "SC's choice =
/// mod default" and means mod updates that introduce new config
/// fields cannot evolve cleanly. The drop pass runs in
/// `apply-curator` and removes matching entries from
/// [`PackConfig::assets`] before [`build`] writes the manifest.
///
/// Paths match the asset `dest` field byte-for-byte (no glob, no
/// regex). One entry per file. Modrinth-sourced assets (extra
/// shaderpacks, resourcepacks) are never matched even if their
/// dest accidentally collides -- the filter keys on
/// `source.type == "smrt_static"` to keep curator extras safe.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct DropAssets {
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Auto-generated artefacts that land in the pack's static area as
/// part of `apply-curator` and become regular `smrt_static` assets in
/// the resulting manifest. Each generator is gated by a boolean so
/// curator opts in per pack.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct GenerateConfig {
    /// Emit `<static>/<filename>` listing the curator-authored
    /// `(lowercase_modid, claimed_version)` pairs verbatim. Adds a
    /// matching `smrt_static` asset entry so the launcher syncs the
    /// file into `<clientDir>/<filename>`. The hidemymods coremod
    /// reads it at FML handshake time to spoof SC's required
    /// modlist.
    ///
    /// CRITICAL: the entries below describe what SC's SERVER
    /// expects, NOT what our pack actually ships. Hivens-rework
    /// packs are already divergent from SC canonical (extra cozy
    /// mods, library swaps, OSN replacing Smarty); auto-extract
    /// from our jars produces the WRONG spoof (SC would receive
    /// our versions instead of its expected ones, and the handshake
    /// would reject). Source of truth is SC's wire ModList,
    /// captured once per SC update and pasted here.
    #[serde(default)]
    pub hidemymods: bool,
    /// Filename inside the static dir. Default
    /// `hidemymods-spoof.json` matches the launcher-side convention
    /// the hidemymods coremod reads from `<clientDir>/`.
    #[serde(default = "default_hidemymods_filename")]
    pub hidemymods_filename: String,
    /// The actual spoof table: `lowercase_modid -> claimed_version`.
    /// Curator-authored from SC's wire ModList; reproduced verbatim
    /// in the emitted JSON. Values like `"$version"` (SC's literal
    /// placeholder for `nbtedit`) round-trip byte-for-byte.
    #[serde(default)]
    pub hidemymods_entries: HashMap<String, String>,
}

fn default_hidemymods_filename() -> String {
    "hidemymods-spoof.json".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct MarkOptional {
    #[serde(default)]
    pub filenames: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct SubstituteEntry {
    pub source: crate::domain::SourceDecl,
    #[serde(default)]
    pub display: Option<Display>,
}

/// Modrinth-direct extra mod the curator wants to add on top of the
/// SC-derived pack. The build pipeline does the Modrinth API lookup
/// at apply time to resolve project_id + version_id + primary file.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ExtraMod {
    pub slug: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub name_override: Option<String>,
}

/// Modrinth-direct extra asset (resourcepack / shaderpack / data pack).
/// `dest_dir` is the destination subfolder ("resourcepacks",
/// "shaderpacks", etc); the resolved filename appends to that path.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ExtraAsset {
    pub slug: String,
    pub dest_dir: String,
    pub modrinth_kind: ExtraAssetKind,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub name_override: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
#[serde(rename_all = "snake_case")]
pub enum ExtraAssetKind {
    /// `https://modrinth.com/resourcepack/<slug>`
    Resourcepack,
    /// `https://modrinth.com/shader/<slug>`
    Shader,
    /// `https://modrinth.com/datapack/<slug>` (rare, included for
    /// completeness)
    Datapack,
}

impl ExtraAssetKind {
    fn modrinth_url_prefix(self) -> &'static str {
        match self {
            ExtraAssetKind::Resourcepack => "https://modrinth.com/resourcepack",
            ExtraAssetKind::Shader => "https://modrinth.com/shader",
            ExtraAssetKind::Datapack => "https://modrinth.com/datapack",
        }
    }
}

fn default_true() -> bool {
    true
}

pub fn load_curator(path: &Path) -> Result<Curator> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading curator file {}", path.display()))?;
    parse_curator(&raw).with_context(|| format!("parsing curator file {}", path.display()))
}

/// Parse curator TOML from a string (the panel edits the raw text). Same
/// shape validation as [`load_curator`] without the file read.
pub fn parse_curator(text: &str) -> Result<Curator> {
    toml::from_str(text).context("parsing curator TOML")
}

/// Merge a structured [`Curator`] into an existing curator.toml, preserving the
/// existing document's comments where toml_edit can (a kept key keeps its
/// leading decor). Each managed table is replaced from the struct; empty
/// tables / arrays are dropped so unused features don't clutter the file. Inner
/// per-line comments inside a replaced table are not preserved -- the raw
/// editor stays the full-fidelity path.
pub fn merge_curator(existing: &str, curator: &Curator) -> Result<String> {
    use toml_edit::DocumentMut;
    let mut doc: DocumentMut = if existing.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing.parse().context("parsing existing curator.toml")?
    };
    let fresh: DocumentMut = toml_edit::ser::to_string(curator)
        .context("serializing curator")?
        .parse()
        .context("re-parsing serialized curator")?;
    for (key, item) in fresh.as_table().iter() {
        if is_empty_item(item) {
            doc.remove(key);
        } else {
            doc[key] = item.clone();
        }
    }
    Ok(doc.to_string())
}

fn is_empty_item(item: &toml_edit::Item) -> bool {
    use toml_edit::{Item, Value};
    match item {
        Item::None => true,
        Item::Table(t) => t.iter().all(|(_, v)| is_empty_item(v)),
        Item::ArrayOfTables(a) => a.is_empty(),
        Item::Value(Value::Array(a)) => a.is_empty(),
        Item::Value(Value::InlineTable(t)) => t.is_empty(),
        Item::Value(_) => false,
    }
}

// ── Mutations (sync) ──────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct MarkOptionalReport {
    pub flipped: u32,
    pub not_found: Vec<String>,
}

/// Flips `required: false` on every mod whose filename appears in
/// [`MarkOptional::filenames`]. Reports filenames that did not match
/// any mod so the curator can spot typos.
pub fn apply_mark_optional(config: &mut PackConfig, mark: &MarkOptional) -> MarkOptionalReport {
    let mut report = MarkOptionalReport::default();
    let names: std::collections::HashSet<&str> =
        mark.filenames.iter().map(|s| s.as_str()).collect();
    let mut hit: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for m in config.mods.iter_mut() {
        if names.contains(m.filename.as_str()) {
            if m.required {
                m.required = false;
                report.flipped += 1;
            }
            hit.insert(m.filename.as_str());
        }
    }
    for name in &mark.filenames {
        if !hit.contains(name.as_str()) {
            report.not_found.push(name.clone());
        }
    }
    report.not_found.sort();
    info!(
        flipped = report.flipped,
        not_found = report.not_found.len(),
        "mark-optional applied"
    );
    report
}

/// Writes curator-declared incompatibilities into each mod's
/// `display.incompatible_with`. The launcher treats incompatibility as mutual,
/// so declaring one direction is enough. Creates a `Display` when absent.
pub fn apply_incompatible(config: &mut PackConfig, incompatible: &HashMap<String, Vec<String>>) {
    if incompatible.is_empty() {
        return;
    }
    let mut applied = 0u32;
    for m in config.mods.iter_mut() {
        if let Some(conflicts) = incompatible.get(&m.filename) {
            let display = m.display.get_or_insert_with(Display::default);
            for c in conflicts {
                if !display.incompatible_with.contains(c) {
                    display.incompatible_with.push(c.clone());
                    applied += 1;
                }
            }
        }
    }
    info!(applied, declared = incompatible.len(), "incompatible applied");
}

/// Flips `default_enabled = false` on each OPTIONAL mod named in `default_off`.
/// Required mods are skipped -- they always install, so the flag is meaningless.
pub fn apply_default_off(config: &mut PackConfig, default_off: &[String]) {
    if default_off.is_empty() {
        return;
    }
    let names: std::collections::HashSet<&str> = default_off.iter().map(|s| s.as_str()).collect();
    let mut flipped = 0u32;
    for m in config.mods.iter_mut() {
        if names.contains(m.filename.as_str()) && !m.required {
            m.default_enabled = false;
            flipped += 1;
        }
    }
    info!(flipped, "default-off applied");
}

#[derive(Debug, Default)]
pub struct SubstituteReport {
    pub applied: u32,
    pub not_found: Vec<String>,
}

/// Replaces the `source` (and optionally `display`) of every mod whose
/// filename matches an entry in [`Curator::substitute`]. The classic
/// use case is swapping SC's proprietary `Smarty-1.12.2.jar` source
/// from the upstream cache to the open-smrt-network jar bytes -- same
/// filename on the wire, different content on disk.
pub fn apply_substitute(
    config: &mut PackConfig,
    substitute: &HashMap<String, SubstituteEntry>,
) -> SubstituteReport {
    let mut report = SubstituteReport::default();
    let mut hit: std::collections::HashSet<String> = std::collections::HashSet::new();
    for m in config.mods.iter_mut() {
        if let Some(sub) = substitute.get(&m.filename) {
            m.source = sub.source.clone();
            if let Some(d) = &sub.display {
                m.display = Some(d.clone());
            }
            report.applied += 1;
            hit.insert(m.filename.clone());
        }
    }
    for fname in substitute.keys() {
        if !hit.contains(fname) {
            report.not_found.push(fname.clone());
        }
    }
    report.not_found.sort();
    info!(
        applied = report.applied,
        not_found = report.not_found.len(),
        "substitute applied"
    );
    report
}

#[derive(Debug, Default)]
pub struct CategoryApplyReport {
    /// Mods that gained a category from the table where none existed.
    pub applied: u32,
    /// Mods whose existing category was OVERWRITTEN by the curator's
    /// table entry. Surfaced separately from `applied` so the operator
    /// can spot when an upstream-derived category (mcmod.info heuristic,
    /// PackConfig literal, etc) is being replaced.
    pub overrode: u32,
    pub unmatched_in_table: Vec<String>,
}

/// Writes `display.category` on every mod matched by the table. The
/// curator's table is AUTHORITATIVE -- it always overrides any
/// pre-existing category from the input PackConfig (bootstrap default,
/// hand-edited literal, mcmod.info heuristic). This is the inverse of
/// the original "skip if already set" rule: in practice the table IS
/// the canonical human-curated assignment, and the bootstrap-derived
/// values it was previously "protecting" turned out to be the wrong
/// thing to preserve.
///
/// To opt out of overriding for a specific entry, the curator simply
/// leaves it out of the table; the existing value stays.
pub fn apply_category_table(
    config: &mut PackConfig,
    table: &HashMap<String, String>,
) -> CategoryApplyReport {
    let mut report = CategoryApplyReport::default();
    let filenames: std::collections::HashSet<&str> =
        config.mods.iter().map(|m| m.filename.as_str()).collect();
    for fname in table.keys() {
        if !filenames.contains(fname.as_str()) {
            report.unmatched_in_table.push(fname.clone());
        }
    }
    report.unmatched_in_table.sort();

    for m in config.mods.iter_mut() {
        let Some(cat) = table.get(&m.filename) else {
            continue;
        };
        let display = m.display.get_or_insert_with(default_display);
        if display.category.as_deref() == Some(cat.as_str()) {
            // Already at the curator's value; nothing to do, no event.
            continue;
        }
        if display.category.is_some() {
            report.overrode += 1;
        } else {
            report.applied += 1;
        }
        display.category = Some(cat.clone());
    }
    info!(
        applied = report.applied,
        overrode = report.overrode,
        unmatched = report.unmatched_in_table.len(),
        "apply-category-table complete"
    );
    report
}

// ── Mutations (async, Modrinth) ───────────────────────────────────────────

#[derive(Debug, Default)]
pub struct ExtraAddReport {
    pub mods_added: u32,
    pub assets_added: u32,
    pub mods_failed: Vec<(String, String)>, // (slug, error)
    pub assets_failed: Vec<(String, String)>,
}

/// Resolves each [`ExtraMod`] / [`ExtraAsset`] via Modrinth and appends
/// the resulting entries to the config. Modrinth lookups use the
/// project's latest 1.12.2 version (we follow MC version of the pack
/// implicitly). On lookup failure the entry is recorded in the report
/// but does not abort the whole apply -- the curator can re-run after
/// fixing the broken slug.
pub async fn apply_extras(
    config: &mut PackConfig,
    modrinth: &super::modrinth::Modrinth,
    extras_mods: &[ExtraMod],
    extras_assets: &[ExtraAsset],
    mc_version: &str,
) -> ExtraAddReport {
    let mut report = ExtraAddReport::default();
    for em in extras_mods {
        match resolve_modrinth_latest_for_mc(modrinth, &em.slug, mc_version).await {
            Ok((project_id, version_id, filename)) => {
                let display = Some(Display {
                    name: em
                        .name_override
                        .clone()
                        .or_else(|| Some(slug_to_title(&em.slug))),
                    description: em.description.clone(),
                    category: em.category.clone(),
                    incompatible_with: Vec::new(),
                    license: None,
                    url: Some(format!("https://modrinth.com/mod/{}", em.slug)),
                    icon_url: None,
                    role: None,
                    requires: Vec::new(),
                });
                config.mods.push(crate::domain::DeclaredMod {
                    filename,
                    required: em.required,
                    default_enabled: true,
                    source: crate::domain::SourceDecl::Modrinth {
                        project_id,
                        version_id,
                    },
                    display,
                    note: Some(format!("added via curator extras: {}", em.slug)),
                });
                report.mods_added += 1;
            }
            Err(e) => report.mods_failed.push((em.slug.clone(), e.to_string())),
        }
    }
    for ea in extras_assets {
        match resolve_modrinth_latest_for_mc(modrinth, &ea.slug, mc_version).await {
            Ok((project_id, version_id, filename)) => {
                let display = Some(Display {
                    name: ea
                        .name_override
                        .clone()
                        .or_else(|| Some(slug_to_title(&ea.slug))),
                    description: ea.description.clone(),
                    category: ea.category.clone(),
                    incompatible_with: Vec::new(),
                    license: None,
                    url: Some(format!(
                        "{}/{}",
                        ea.modrinth_kind.modrinth_url_prefix(),
                        ea.slug
                    )),
                    icon_url: None,
                    role: None,
                    requires: Vec::new(),
                });
                let dest = format!("{}/{}", ea.dest_dir.trim_end_matches('/'), filename);
                config.assets.push(crate::domain::DeclaredAsset {
                    dest,
                    required: ea.required,
                    source: crate::domain::SourceDecl::Modrinth {
                        project_id,
                        version_id,
                    },
                    display,
                    note: Some(format!("added via curator extras: {}", ea.slug)),
                });
                report.assets_added += 1;
            }
            Err(e) => report.assets_failed.push((ea.slug.clone(), e.to_string())),
        }
    }
    info!(
        mods_added = report.mods_added,
        assets_added = report.assets_added,
        mods_failed = report.mods_failed.len(),
        assets_failed = report.assets_failed.len(),
        "extras applied"
    );
    report
}

/// Looks up the most recent Modrinth version for [slug] that lists
/// [mc_version] in its game_versions. Returns (project_id,
/// version_id, primary filename). Fails when the project has no
/// matching version.
async fn resolve_modrinth_latest_for_mc(
    modrinth: &super::modrinth::Modrinth,
    slug: &str,
    mc_version: &str,
) -> Result<(String, String, String)> {
    let versions = modrinth
        .project_versions(slug, Some(mc_version))
        .await
        .with_context(|| format!("listing Modrinth versions for {slug}"))?;
    let v = versions
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no Modrinth version of {slug} matches mc={mc_version}"))?;
    let f = v
        .primary_file()
        .ok_or_else(|| anyhow::anyhow!("Modrinth version {} of {slug} has no files", v.id))?
        .clone();
    Ok((v.project_id, v.id, f.filename))
}

fn slug_to_title(slug: &str) -> String {
    slug.split('-')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Pass 4: generate hidemymods spoof JSON ────────────────────────────────

/// Per-mod entry emitted into the hidemymods spoof file. Wire shape
/// is `{"id": "<lowercase_modid>", "version": "<string>"}`. Match
/// SC's own format byte-for-byte so a curl-diff against the SC
/// snapshot is a clean compare.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HidemymodsEntry {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HidemymodsSpoof {
    /// Underscore-prefixed so jq filters by `.mods` still work; the
    /// note is documentation for whoever opens the file by hand,
    /// not a schema field.
    #[serde(rename = "_note")]
    pub note: String,
    pub mods: Vec<HidemymodsEntry>,
}

#[derive(Debug, Default)]
pub struct HidemymodsReport {
    pub entries_emitted: u32,
    pub asset_entry_added: bool,
}

/// Emits `<storage>/packs/<pack_id>/static/<filename>` containing the
/// curator-authored `(lowercase_modid, claimed_version)` table from
/// `generate_cfg.hidemymods_entries` verbatim, plus a matching
/// `smrt_static` `DeclaredAsset` entry on the config so the launcher
/// syncs the file into `<clientDir>/<filename>`.
///
/// The entries describe SC's expected ModList (what the server's FML
/// handshake check requires) and not the contents of our pack. A
/// Hivens-rework pack ships a divergent set (extra cozy mods, OSN
/// replacing Smarty, library swaps), and the spoof must claim SC's
/// values so the handshake accepts. An earlier revision auto-extracted
/// modid plus version from each jar's mcmod.info; that produced the
/// WRONG answer for divergent mods (SC kicks: client claims our
/// version, server expected its version, mismatch). Spoof is now an
/// authoritative curator artefact whose source of truth is whatever
/// SC currently sends on the wire, observed once per SC update and
/// pasted into curator.toml.
///
/// Idempotent: re-running over a config that already contains the
/// asset entry does not duplicate it; the JSON file is rewritten.
/// Doesn't touch storage's cache/ tree -- the generator is pure
/// curator-table-to-JSON; nothing depends on jars being present.
pub fn generate_hidemymods_spoof(
    config: &mut PackConfig,
    generate_cfg: &GenerateConfig,
    storage: &Path,
) -> Result<HidemymodsReport> {
    let mut report = HidemymodsReport::default();
    if !generate_cfg.hidemymods {
        return Ok(report);
    }

    let mut entries: Vec<HidemymodsEntry> = generate_cfg
        .hidemymods_entries
        .iter()
        .map(|(modid, version)| HidemymodsEntry {
            id: modid.trim().to_lowercase(),
            version: version.clone(),
        })
        .collect();
    // Stable ordering so diffs across runs are reviewable.
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    report.entries_emitted = entries.len() as u32;

    let spoof = HidemymodsSpoof {
        note: "Generated by smrt-pack apply-curator from the curator-authored \
               hidemymods_entries table. Keys are lowercase Forge mod-IDs from SC's \
               wire ModList; values are the version strings SC sends for each. The \
               hidemymods coremod reads this at FML handshake time and rewrites the \
               wire ModList to claim these values regardless of what the client \
               actually loaded -- the bridge that lets a Hivens-rework pack diverge \
               from SC canonical without breaking the server-side mod-list check."
            .to_string(),
        mods: entries,
    };
    let static_dir = storage.join("packs").join(&config.pack_id).join("static");
    fs::create_dir_all(&static_dir)
        .with_context(|| format!("creating static dir {}", static_dir.display()))?;
    let out_path = static_dir.join(&generate_cfg.hidemymods_filename);
    let tmp_path = out_path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(&spoof)?;
    fs::write(&tmp_path, json).with_context(|| format!("writing {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &out_path)
        .with_context(|| format!("renaming {} to {}", tmp_path.display(), out_path.display()))?;

    // Add the asset entry if missing; rewrite source/display every
    // time so curator-side renames or category changes propagate
    // without manual cleanup.
    let asset_already_present = config
        .assets
        .iter()
        .any(|a| a.dest == generate_cfg.hidemymods_filename);
    if !asset_already_present {
        config.assets.push(crate::domain::DeclaredAsset {
            dest: generate_cfg.hidemymods_filename.clone(),
            required: true,
            source: SourceDecl::SmrtStatic {
                rel_path: generate_cfg.hidemymods_filename.clone(),
            },
            display: Some(Display {
                name: Some("Hidemymods spoof".into()),
                description: Some(
                    "Auto-generated modid -> claimed-version map read by the hidemymods \
                     coremod at FML handshake time. Lets the mirror pack diverge from SC \
                     canonical without breaking the server-side mod-list check."
                        .into(),
                ),
                category: Some("client-defaults".into()),
                incompatible_with: Vec::new(),
                license: None,
                url: Some("https://github.com/Kitty-Hivens/hidemymods".into()),
                icon_url: None,
                role: None,
                requires: Vec::new(),
            }),
            note: Some("generated by smrt-pack apply-curator's generate.hidemymods pass".into()),
        });
        report.asset_entry_added = true;
    }

    info!(
        entries = report.entries_emitted,
        new_asset = report.asset_entry_added,
        "generate-hidemymods-spoof complete"
    );
    Ok(report)
}

// ── Orchestrator ──────────────────────────────────────────────────────────

/// Runs the full curator chain on [config] in the canonical order:
///   1. enrich from mcmod.info     (synchronous, file IO)
///   2. apply role_table            (sync)
///   3. apply category_table        (sync)
///   4. mark optional               (sync)
///   5. substitute sources          (sync)
///   6. infer requires from mcmod   (sync, file IO)
///   7. drop curator-rejected smrt_static assets (sync)
///   8. generate hidemymods spoof   (sync)
///   9. add extras (mods + assets)  (async, Modrinth)
///
/// Order matters: substitutes happen BEFORE infer_requires so the
/// substituted jar's mcmod.info (open-smrt-network's, not the
/// upstream proprietary jar's) feeds the dep graph. The drop pass
/// runs AFTER substitute / mark-optional / category-table so a
/// dropped entry cannot accidentally short-circuit those mutations
/// for a sibling file. Extras land last so their display.category
/// does not leak into category_table resolution against SC-derived
/// mods.
pub async fn apply_curator(
    config: &mut PackConfig,
    curator: &Curator,
    storage: &Path,
    modrinth: &super::modrinth::Modrinth,
    mc_version: &str,
) -> Result<()> {
    enrich_from_mcmod_info(config, storage)?;
    apply_role_table(
        config,
        &RoleTable {
            roles: curator.role_table.clone(),
        },
    )?;
    apply_category_table(config, &curator.category_table);
    apply_mark_optional(config, &curator.mark_optional);
    apply_default_off(config, &curator.default_off);
    apply_incompatible(config, &curator.incompatible);
    apply_substitute(config, &curator.substitute);
    infer_requires_from_mcmod_info(config, storage)?;
    apply_drop_assets(config, &curator.drop_assets);
    // Hidemymods generation runs AFTER drop_assets so a hand-curated
    // spoof filename that was previously generated and then declared
    // unwanted via drop_assets still gets re-emitted (the generator
    // re-adds the asset entry every run). Conversely, generating
    // BEFORE drop_assets would let a stale drop_assets entry strip
    // the spoof we just produced -- the opposite of what curator
    // wants.
    generate_hidemymods_spoof(config, &curator.generate, storage)?;
    apply_extras(
        config,
        modrinth,
        &curator.extra_mods,
        &curator.extra_assets,
        mc_version,
    )
    .await;
    Ok(())
}

#[derive(Debug, Default)]
pub struct DropAssetsReport {
    /// Number of asset entries actually removed.
    pub dropped: u32,
    /// Curator-declared paths that did not match any asset entry --
    /// surfaced so the curator can spot typos / stale drop lists
    /// after a bootstrap layout change.
    pub not_found: Vec<String>,
    /// Curator-declared paths that matched a non-smrt_static asset
    /// (Modrinth-sourced extra, etc) and were intentionally skipped.
    /// Reported so the curator notices when a drop entry hits an
    /// unexpected source type.
    pub skipped_non_static: Vec<String>,
}

/// Strips `smrt_static` asset entries whose `dest` appears in
/// [`DropAssets::paths`]. Modrinth-sourced and smrt_cache-sourced
/// assets are NEVER removed even if their dest collides -- the
/// filter is intentionally narrow because extras (resource packs,
/// shaders added via curator's `extra_assets`) live in the same
/// `assets[]` array and a too-broad filter could nuke them by
/// accident.
///
/// Idempotent: re-running with the same drop list against a
/// post-drop config simply reports `dropped=0`. Safe to call from
/// the orchestrator on every apply-curator run.
pub fn apply_drop_assets(config: &mut PackConfig, drop: &DropAssets) -> DropAssetsReport {
    let mut report = DropAssetsReport::default();
    if drop.paths.is_empty() {
        return report;
    }
    let drop_set: std::collections::HashSet<&str> = drop.paths.iter().map(|s| s.as_str()).collect();

    // First pass: figure out which declared paths are present and
    // under which source type. Used for the not_found / skipped
    // reports so the operator sees both classes of mismatch.
    let mut hit_static: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut hit_non_static: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for a in &config.assets {
        if let Some(s) = drop_set.get(a.dest.as_str()) {
            match &a.source {
                SourceDecl::SmrtStatic { .. } => {
                    hit_static.insert(*s);
                }
                _ => {
                    hit_non_static.insert(*s);
                }
            }
        }
    }

    let before = config.assets.len();
    config.assets.retain(|a| match &a.source {
        SourceDecl::SmrtStatic { .. } => !drop_set.contains(a.dest.as_str()),
        _ => true,
    });
    report.dropped = (before - config.assets.len()) as u32;

    for p in &drop.paths {
        if !hit_static.contains(p.as_str()) {
            if hit_non_static.contains(p.as_str()) {
                report.skipped_non_static.push(p.clone());
            } else {
                report.not_found.push(p.clone());
            }
        }
    }
    report.not_found.sort();
    report.skipped_non_static.sort();

    info!(
        dropped = report.dropped,
        not_found = report.not_found.len(),
        skipped_non_static = report.skipped_non_static.len(),
        "apply-drop-assets complete"
    );
    report
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
/// that did not match any mod so the curator can spot typos.
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
/// Existing `display.requires` wins -- this pass never replaces a
/// curator-authored list, only fills an empty one.
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

fn cache_jar_path(storage: &Path, sha1: &str) -> Result<PathBuf> {
    if sha1.len() != 40 || !sha1.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("invalid sha1: {sha1}");
    }
    let prefix = &sha1[..2];
    Ok(storage
        .join("cache")
        .join(prefix)
        .join(format!("{sha1}.jar")))
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DeclaredMod, PackConfig, SourceDecl};
    use crate::domain::LoaderSpec;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;

    #[test]
    fn merge_curator_replaces_values_drops_empty_and_keeps_comments() {
        let existing = "# top-of-file note\ndefault_off = [\"A.jar\"]\n\n# category notes\n[category_table]\n\"A.jar\" = \"old\"\n";
        let mut c = Curator {
            default_off: vec!["B.jar".to_string()],
            ..Default::default()
        };
        c.category_table.insert("A.jar".to_string(), "new".to_string());
        let merged = merge_curator(existing, &c).unwrap();
        // values updated
        assert!(merged.contains("B.jar"), "default_off replaced: {merged}");
        assert!(merged.contains("\"new\""), "category replaced: {merged}");
        assert!(!merged.contains("\"old\""), "old value gone: {merged}");
        // unused/default tables are not emitted
        assert!(!merged.contains("[mark_optional]"), "empty table dropped: {merged}");
        assert!(!merged.contains("[generate]"), "empty table dropped: {merged}");
        // round-trips as a valid Curator
        parse_curator(&merged).unwrap();
        // the doc-level comment survives the merge
        assert!(merged.contains("# top-of-file note"), "doc comment survives: {merged}");
    }

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
            tags: Vec::new(),
            featured: false,
            mods: Vec::new(),
            assets: Vec::new(),
        }
    }

    fn write_test_jar(dir: &Path, sha1: &str, mcmod_json: &str) -> Result<()> {
        let prefix = &sha1[..2];
        let cache_dir = dir.join("cache").join(prefix);
        fs::create_dir_all(&cache_dir)?;
        let jar_path = cache_dir.join(format!("{sha1}.jar"));
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
    fn mark_optional_flips_required_flag_only_for_matched() {
        let mut cfg = empty_config();
        for fname in ["BetterChat.jar", "AlwaysRequired.jar"] {
            cfg.mods.push(DeclaredMod {
                filename: fname.into(),
                required: true,
                default_enabled: true,
                source: SourceDecl::SmrtCache {
                    sha1: "a".repeat(40),
                },
                display: None,
                note: None,
            });
        }
        let mark = MarkOptional {
            filenames: vec!["BetterChat.jar".into(), "Typo.jar".into()],
        };
        let report = apply_mark_optional(&mut cfg, &mark);
        assert_eq!(report.flipped, 1);
        assert_eq!(report.not_found, vec!["Typo.jar".to_string()]);
        assert!(!cfg.mods[0].required); // BetterChat flipped
        assert!(cfg.mods[1].required); // AlwaysRequired untouched
    }

    #[test]
    fn default_off_flips_optionals_and_incompatible_writes_display() {
        let mut cfg = empty_config();
        cfg.mods.push(DeclaredMod {
            filename: "FoamFix.jar".into(),
            required: false,
            default_enabled: true,
            source: SourceDecl::SmrtCache { sha1: "a".repeat(40) },
            display: None,
            note: None,
        });
        cfg.mods.push(DeclaredMod {
            filename: "Mixinbooter.jar".into(),
            required: true,
            default_enabled: true,
            source: SourceDecl::SmrtCache { sha1: "b".repeat(40) },
            display: None,
            note: None,
        });

        // Naming a required mod in default_off is a no-op (required always on).
        apply_default_off(&mut cfg, &["FoamFix.jar".into(), "Mixinbooter.jar".into()]);
        let mut incompat: HashMap<String, Vec<String>> = HashMap::new();
        incompat.insert("FoamFix.jar".into(), vec!["Mixinbooter.jar".into()]);
        apply_incompatible(&mut cfg, &incompat);

        let foam = cfg.mods.iter().find(|m| m.filename == "FoamFix.jar").unwrap();
        let mixin = cfg.mods.iter().find(|m| m.filename == "Mixinbooter.jar").unwrap();
        assert!(!foam.default_enabled, "optional FoamFix flipped default-off");
        assert!(mixin.default_enabled, "required mod untouched by default-off");
        assert_eq!(
            foam.display.as_ref().unwrap().incompatible_with,
            vec!["Mixinbooter.jar".to_string()],
        );
    }

    #[test]
    fn substitute_swaps_source_and_display() {
        let mut cfg = empty_config();
        cfg.mods.push(DeclaredMod {
            filename: "Smarty-1.12.2.jar".into(),
            required: true,
            default_enabled: true,
            source: SourceDecl::SmrtCache {
                sha1: "0".repeat(40),
            },
            display: None,
            note: None,
        });
        let mut substitute: HashMap<String, SubstituteEntry> = HashMap::new();
        substitute.insert(
            "Smarty-1.12.2.jar".into(),
            SubstituteEntry {
                source: SourceDecl::SmrtCache {
                    sha1: "f".repeat(40),
                },
                display: Some(Display {
                    name: Some("Open Smarty Network".into()),
                    description: None,
                    category: Some("lib".into()),
                    incompatible_with: Vec::new(),
                    license: Some("Apache-2.0".into()),
                    url: None,
                    icon_url: None,
                    role: None,
                    requires: Vec::new(),
                }),
            },
        );
        let report = apply_substitute(&mut cfg, &substitute);
        assert_eq!(report.applied, 1);
        assert!(report.not_found.is_empty());
        match &cfg.mods[0].source {
            SourceDecl::SmrtCache { sha1 } => assert_eq!(sha1, &"f".repeat(40)),
            other => panic!("expected SmrtCache after substitute, got {other:?}"),
        }
        assert_eq!(
            cfg.mods[0].display.as_ref().unwrap().name.as_deref(),
            Some("Open Smarty Network")
        );
    }

    #[test]
    fn category_table_overrides_existing_and_fills_missing() {
        // Three mods: one with no category (table fills), one with a
        // category the table replaces (curator wins), one with a
        // category the table happens to match (no-op, no event).
        let mut cfg = empty_config();
        cfg.mods.push(DeclaredMod {
            filename: "Modded.jar".into(),
            required: true,
            default_enabled: true,
            source: SourceDecl::SmrtCache {
                sha1: "a".repeat(40),
            },
            display: None,
            note: None,
        });
        cfg.mods.push(DeclaredMod {
            filename: "PreviouslyCore.jar".into(),
            required: true,
            default_enabled: true,
            source: SourceDecl::SmrtCache {
                sha1: "a".repeat(40),
            },
            display: Some(Display {
                name: None,
                description: None,
                category: Some("core".into()),
                incompatible_with: Vec::new(),
                license: None,
                url: None,
                icon_url: None,
                role: None,
                requires: Vec::new(),
            }),
            note: None,
        });
        cfg.mods.push(DeclaredMod {
            filename: "AlreadyRight.jar".into(),
            required: true,
            default_enabled: true,
            source: SourceDecl::SmrtCache {
                sha1: "a".repeat(40),
            },
            display: Some(Display {
                name: None,
                description: None,
                category: Some("performance".into()),
                incompatible_with: Vec::new(),
                license: None,
                url: None,
                icon_url: None,
                role: None,
                requires: Vec::new(),
            }),
            note: None,
        });
        let mut table = HashMap::new();
        table.insert("Modded.jar".into(), "performance".into());
        table.insert("PreviouslyCore.jar".into(), "lib".into());
        table.insert("AlreadyRight.jar".into(), "performance".into());
        let report = apply_category_table(&mut cfg, &table);
        assert_eq!(report.applied, 1, "Modded.jar got its first category");
        assert_eq!(
            report.overrode, 1,
            "PreviouslyCore.jar core->lib counted as override"
        );
        // AlreadyRight.jar already matched the table; no event.
        assert_eq!(
            cfg.mods[0].display.as_ref().unwrap().category.as_deref(),
            Some("performance")
        );
        assert_eq!(
            cfg.mods[1].display.as_ref().unwrap().category.as_deref(),
            Some("lib"),
            "curator table wins over pre-existing 'core'",
        );
        assert_eq!(
            cfg.mods[2].display.as_ref().unwrap().category.as_deref(),
            Some("performance")
        );
    }

    #[test]
    fn generate_hidemymods_disabled_is_noop() {
        let dir = TempDir::new().unwrap();
        let mut cfg = empty_config();
        let g = GenerateConfig::default(); // hidemymods = false
        let report = generate_hidemymods_spoof(&mut cfg, &g, dir.path()).unwrap();
        assert_eq!(report.entries_emitted, 0);
        assert!(!report.asset_entry_added);
        assert!(cfg.assets.is_empty());
        assert!(
            !dir.path()
                .join("packs/Test/static/hidemymods-spoof.json")
                .exists()
        );
    }

    #[test]
    fn generate_hidemymods_emits_curator_entries_verbatim() {
        // Reads the curator-authored table directly; does NOT walk
        // jars. Two entries go in, two come out -- in lowercase + sort
        // order -- regardless of what's in config.mods (in fact the
        // config has zero mods here, to prove the generator is jar-
        // independent).
        let dir = TempDir::new().unwrap();
        let mut entries = HashMap::new();
        entries.insert("buildcraftcore".into(), "7.99.24.6".into());
        entries.insert("appliedenergistics2".into(), "rv6-stable-7".into());

        let mut cfg = empty_config();
        let g = GenerateConfig {
            hidemymods: true,
            hidemymods_filename: "hidemymods-spoof.json".into(),
            hidemymods_entries: entries,
        };
        let report = generate_hidemymods_spoof(&mut cfg, &g, dir.path()).unwrap();
        assert_eq!(report.entries_emitted, 2);
        assert!(report.asset_entry_added);
        assert_eq!(cfg.assets.len(), 1);
        assert_eq!(cfg.assets[0].dest, "hidemymods-spoof.json");

        let spoof_path = dir.path().join("packs/Test/static/hidemymods-spoof.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(spoof_path).unwrap()).unwrap();
        let mods = parsed["mods"].as_array().unwrap();
        // Sorted by id: appliedenergistics2 before buildcraftcore.
        assert_eq!(mods[0]["id"], "appliedenergistics2");
        assert_eq!(mods[0]["version"], "rv6-stable-7");
        assert_eq!(mods[1]["id"], "buildcraftcore");
        assert_eq!(mods[1]["version"], "7.99.24.6");
    }

    #[test]
    fn generate_hidemymods_round_trips_placeholder_versions() {
        // SC sends literal "$version" for nbtedit (their build script
        // never substituted the Gradle placeholder, but the wire
        // ModList carries it as-is). Spoof must emit it byte-for-byte
        // since hidemymods replays whatever's in the JSON.
        let dir = TempDir::new().unwrap();
        let mut entries = HashMap::new();
        entries.insert("nbtedit".into(), "$version".into());
        entries.insert("smarty".into(), "1.12.2".into());

        let mut cfg = empty_config();
        let g = GenerateConfig {
            hidemymods: true,
            hidemymods_filename: "hidemymods-spoof.json".into(),
            hidemymods_entries: entries,
        };
        generate_hidemymods_spoof(&mut cfg, &g, dir.path()).unwrap();
        let path = dir.path().join("packs/Test/static/hidemymods-spoof.json");
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let mods = v["mods"].as_array().unwrap();
        let nbt = mods.iter().find(|e| e["id"] == "nbtedit").unwrap();
        assert_eq!(nbt["version"], "$version");
        let smarty = mods.iter().find(|e| e["id"] == "smarty").unwrap();
        assert_eq!(smarty["version"], "1.12.2");
    }

    #[test]
    fn generate_hidemymods_idempotent_no_duplicate_asset() {
        let dir = TempDir::new().unwrap();
        let mut entries = HashMap::new();
        entries.insert("x".into(), "1".into());
        let mut cfg = empty_config();
        let g = GenerateConfig {
            hidemymods: true,
            hidemymods_filename: "hidemymods-spoof.json".into(),
            hidemymods_entries: entries,
        };

        let r1 = generate_hidemymods_spoof(&mut cfg, &g, dir.path()).unwrap();
        assert!(r1.asset_entry_added);
        let r2 = generate_hidemymods_spoof(&mut cfg, &g, dir.path()).unwrap();
        assert!(
            !r2.asset_entry_added,
            "second run must not duplicate the asset entry"
        );
        assert_eq!(cfg.assets.len(), 1);
    }

    #[test]
    fn generate_hidemymods_lowercases_modids() {
        // SC wire IDs are always lowercase, but a sloppy curator
        // edit could mix cases; the generator normalises so the
        // spoof always matches SC's casing.
        let dir = TempDir::new().unwrap();
        let mut entries = HashMap::new();
        entries.insert("AppliedEnergistics2".into(), "rv6".into());
        let mut cfg = empty_config();
        let g = GenerateConfig {
            hidemymods: true,
            hidemymods_filename: "hidemymods-spoof.json".into(),
            hidemymods_entries: entries,
        };
        generate_hidemymods_spoof(&mut cfg, &g, dir.path()).unwrap();
        let path = dir.path().join("packs/Test/static/hidemymods-spoof.json");
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(v["mods"][0]["id"], "appliedenergistics2");
    }

    #[test]
    fn drop_assets_strips_only_smrt_static_matches() {
        // Three assets, all with `config/foo.cfg` shape -- one
        // smrt_static, one Modrinth, one smrt_cache. Only the
        // smrt_static one must disappear.
        let mut cfg = empty_config();
        cfg.assets.push(crate::domain::DeclaredAsset {
            dest: "config/foamfix.cfg".into(),
            required: true,
            source: SourceDecl::SmrtStatic {
                rel_path: "config/foamfix.cfg".into(),
            },
            display: None,
            note: None,
        });
        cfg.assets.push(crate::domain::DeclaredAsset {
            dest: "config/quark.cfg".into(),
            required: true,
            source: SourceDecl::SmrtStatic {
                rel_path: "config/quark.cfg".into(),
            },
            display: None,
            note: None,
        });
        cfg.assets.push(crate::domain::DeclaredAsset {
            dest: "resourcepacks/Better-Farm-Animals.zip".into(),
            required: false,
            source: SourceDecl::Modrinth {
                project_id: "abc".into(),
                version_id: "xyz".into(),
            },
            display: None,
            note: None,
        });
        cfg.assets.push(crate::domain::DeclaredAsset {
            dest: "shaderpacks/mellow.zip".into(),
            required: false,
            source: SourceDecl::SmrtCache {
                sha1: "f".repeat(40),
            },
            display: None,
            note: None,
        });

        let drop = DropAssets {
            paths: vec![
                "config/foamfix.cfg".into(),                    // hits smrt_static
                "resourcepacks/Better-Farm-Animals.zip".into(), // hits Modrinth, must be skipped
                "shaderpacks/mellow.zip".into(),                // hits smrt_cache, must be skipped
                "config/never-existed.cfg".into(),              // not_found
            ],
        };
        let report = apply_drop_assets(&mut cfg, &drop);
        assert_eq!(report.dropped, 1);
        assert_eq!(
            cfg.assets.len(),
            3,
            "only the smrt_static entry must disappear"
        );
        assert!(
            cfg.assets.iter().all(|a| a.dest != "config/foamfix.cfg"),
            "foamfix.cfg should be gone",
        );
        assert_eq!(
            report.skipped_non_static,
            vec![
                "resourcepacks/Better-Farm-Animals.zip".to_string(),
                "shaderpacks/mellow.zip".to_string(),
            ],
        );
        assert_eq!(
            report.not_found,
            vec!["config/never-existed.cfg".to_string()]
        );
    }

    #[test]
    fn drop_assets_is_idempotent() {
        let mut cfg = empty_config();
        cfg.assets.push(crate::domain::DeclaredAsset {
            dest: "config/foamfix.cfg".into(),
            required: true,
            source: SourceDecl::SmrtStatic {
                rel_path: "config/foamfix.cfg".into(),
            },
            display: None,
            note: None,
        });
        let drop = DropAssets {
            paths: vec!["config/foamfix.cfg".into()],
        };
        let r1 = apply_drop_assets(&mut cfg, &drop);
        assert_eq!(r1.dropped, 1);
        let r2 = apply_drop_assets(&mut cfg, &drop);
        assert_eq!(r2.dropped, 0);
        assert_eq!(r2.not_found, vec!["config/foamfix.cfg".to_string()]);
    }

    #[test]
    fn drop_assets_empty_list_is_noop() {
        let mut cfg = empty_config();
        cfg.assets.push(crate::domain::DeclaredAsset {
            dest: "config/whatever.cfg".into(),
            required: true,
            source: SourceDecl::SmrtStatic {
                rel_path: "config/whatever.cfg".into(),
            },
            display: None,
            note: None,
        });
        let report = apply_drop_assets(&mut cfg, &DropAssets::default());
        assert_eq!(report.dropped, 0);
        assert_eq!(cfg.assets.len(), 1);
    }

    #[test]
    fn industrial_curator_parses_with_full_hidemymods_table() {
        // Worked-example file in examples/industrial/curator.toml is
        // the canonical reference for curator authors. Catch shape
        // drift (e.g. accidentally renaming hidemymods_entries) here
        // instead of at the next live build.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("industrial")
            .join("curator.toml");
        let curator = load_curator(&path).expect("Industrial curator.toml must parse");
        assert!(
            curator.generate.hidemymods,
            "hidemymods flag must be on in the worked example"
        );
        // Snapshot dated 2026-05-26 lists 56 entries. If SC bumps the
        // pack and the table grows or shrinks, bump the expectation
        // -- the assertion is a "did anything fall off the table"
        // guard, not a permanent magic number.
        assert_eq!(
            curator.generate.hidemymods_entries.len(),
            56,
            "expected the full SC Industrial wire ModList in the worked example"
        );
        // Spot-check a few load-bearing entries the spoof has to get
        // exactly right: the literal $version placeholder, the
        // OSN-substituted smarty modid, an AE2 family member.
        assert_eq!(
            curator.generate.hidemymods_entries.get("nbtedit"),
            Some(&"$version".to_string()),
            "nbtedit must be the literal placeholder, not a substituted version"
        );
        assert_eq!(
            curator.generate.hidemymods_entries.get("smarty"),
            Some(&"1.12.2".to_string())
        );
        assert_eq!(
            curator
                .generate
                .hidemymods_entries
                .get("appliedenergistics2"),
            Some(&"rv6-stable-7".to_string())
        );
        // drop_assets list -- 76 paths as of the 2026-05-26 sweep.
        // Bump this expectation when the curator extends the drop
        // table; assertion is a "did anything fall off the worked
        // example" guard, not a magic number.
        assert_eq!(
            curator.drop_assets.paths.len(),
            76,
            "expected the full Industrial drop_assets table in the worked example",
        );
        assert!(
            curator
                .drop_assets
                .paths
                .contains(&"config/Smarty.cfg".to_string()),
            "Smarty.cfg must be in drops -- OSN replaces Smarty and ignores this config",
        );
        assert!(
            curator
                .drop_assets
                .paths
                .contains(&"config/AppliedEnergistics2/items.csv".to_string()),
        );
        assert!(
            curator
                .drop_assets
                .paths
                .contains(&"config/jeresources/world-gen.json".to_string()),
        );
    }

    #[test]
    fn slug_to_title_handles_normal_cases() {
        assert_eq!(slug_to_title("appleskin"), "Appleskin");
        assert_eq!(slug_to_title("better-farm-animals"), "Better Farm Animals");
        assert_eq!(slug_to_title("crafting-tweaks"), "Crafting Tweaks");
        assert_eq!(slug_to_title(""), "");
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
}
