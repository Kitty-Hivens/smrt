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

use crate::pack_config::{PackConfig, SourceDecl};
use crate::types::{Display, Requirement};
use anyhow::{Context, Result};
use serde::Deserialize;
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
#[derive(Debug, Clone, Deserialize)]
pub struct McModInfo {
    #[serde(default)]
    pub modid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Default, Deserialize)]
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
#[derive(Debug, Clone, Default, Deserialize)]
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
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Curator {
    #[serde(default)]
    pub pack_meta: PackMeta,
    #[serde(default)]
    pub mark_optional: MarkOptional,
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
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MarkOptional {
    #[serde(default)]
    pub filenames: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubstituteEntry {
    pub source: crate::pack_config::SourceDecl,
    #[serde(default)]
    pub display: Option<Display>,
}

/// Modrinth-direct extra mod the curator wants to add on top of the
/// SC-derived pack. The build pipeline does the Modrinth API lookup
/// at apply time to resolve project_id + version_id + primary file.
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Copy, Deserialize)]
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
    toml::from_str(&raw).with_context(|| format!("parsing curator file {}", path.display()))
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
    pub applied: u32,
    pub skipped_already_set: u32,
    pub unmatched_in_table: Vec<String>,
}

/// Writes `display.category` on every mod matched by the table.
/// Existing categories win -- this only fills missing ones. Mirrors
/// [`apply_role_table`] in shape.
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
        if display.category.is_some() {
            report.skipped_already_set += 1;
            continue;
        }
        display.category = Some(cat.clone());
        report.applied += 1;
    }
    info!(
        applied = report.applied,
        skipped = report.skipped_already_set,
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
    modrinth: &crate::modrinth::Modrinth,
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
                config.mods.push(crate::pack_config::DeclaredMod {
                    filename,
                    required: em.required,
                    source: crate::pack_config::SourceDecl::Modrinth {
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
                config.assets.push(crate::pack_config::DeclaredAsset {
                    dest,
                    required: ea.required,
                    source: crate::pack_config::SourceDecl::Modrinth {
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
    modrinth: &crate::modrinth::Modrinth,
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

// ── Orchestrator ──────────────────────────────────────────────────────────

/// Runs the full curator chain on [config] in the canonical order:
///   1. enrich from mcmod.info     (synchronous, file IO)
///   2. apply role_table            (sync)
///   3. apply category_table        (sync)
///   4. mark optional               (sync)
///   5. substitute sources          (sync)
///   6. infer requires from mcmod   (sync, file IO)
///   7. add extras (mods + assets)  (async, Modrinth)
///
/// Order matters: substitutes happen BEFORE infer_requires so the
/// substituted jar's mcmod.info (open-smrt-network's, not the
/// upstream proprietary jar's) feeds the dep graph. Extras land last
/// so their display.category does not leak into category_table
/// resolution against SC-derived mods.
pub async fn apply_curator(
    config: &mut PackConfig,
    curator: &Curator,
    storage: &Path,
    modrinth: &crate::modrinth::Modrinth,
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
    apply_substitute(config, &curator.substitute);
    infer_requires_from_mcmod_info(config, storage)?;
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
    use crate::pack_config::{DeclaredMod, PackConfig, SourceDecl};
    use crate::types::LoaderSpec;
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
            source: SourceDecl::SmrtCache { sha1: sha_jei },
            display: None,
            note: None,
        });
        cfg.mods.push(DeclaredMod {
            filename: "JEIAddon.jar".into(),
            required: true,
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
    fn substitute_swaps_source_and_display() {
        let mut cfg = empty_config();
        cfg.mods.push(DeclaredMod {
            filename: "Smarty-1.12.2.jar".into(),
            required: true,
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
    fn category_table_fills_missing_only() {
        let mut cfg = empty_config();
        cfg.mods.push(DeclaredMod {
            filename: "Modded.jar".into(),
            required: true,
            source: SourceDecl::SmrtCache {
                sha1: "a".repeat(40),
            },
            display: None,
            note: None,
        });
        cfg.mods.push(DeclaredMod {
            filename: "WithCategory.jar".into(),
            required: true,
            source: SourceDecl::SmrtCache {
                sha1: "a".repeat(40),
            },
            display: Some(Display {
                name: None,
                description: None,
                category: Some("already-set".into()),
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
        table.insert("WithCategory.jar".into(), "should-not-override".into());
        let report = apply_category_table(&mut cfg, &table);
        assert_eq!(report.applied, 1);
        assert_eq!(report.skipped_already_set, 1);
        assert_eq!(
            cfg.mods[0].display.as_ref().unwrap().category.as_deref(),
            Some("performance")
        );
        assert_eq!(
            cfg.mods[1].display.as_ref().unwrap().category.as_deref(),
            Some("already-set")
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
