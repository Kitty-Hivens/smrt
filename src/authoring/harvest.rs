//! Registry harvest: scan the cache + published manifests, read each jar's
//! `mcmod.info`, batch-resolve Modrinth identity, and reconcile it all into the
//! registry. The scan (FS + network) is async; the write (`write_scan`) is a
//! pure, transactional DB step that's unit-tested without I/O.
//!
//! Phase 1 is harvest-only: it writes `source = harvested | jar-meta | curator`
//! rows and never clobbers authored ones. Modrinth project-level deps are not
//! harvested yet (their target is a project_id, a different selector namespace
//! than the modid relations here) -- that lands with the Phase 4 resolver.

use super::bytecode;
use super::curator::{JarFacts, McModInfo, clean_mc_version, jar_facts, read_mcmod_info};
use super::modrinth::Modrinth;
use crate::registry::model::{RelKind, Source};
use crate::registry::{Registry, queries, upsert};
use crate::storage::Storage;
use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A cached/manifested jar reduced to the facts the registry needs.
pub struct JarSeed {
    pub sha1: String,
    pub size_bytes: i64,
    pub modid: Option<String>,
    pub version: Option<String>,
    pub project_id: Option<String>,
    pub loaders: Vec<String>,
    pub mc_versions: Vec<String>,
    pub requires: Vec<String>, // jar-meta dep modids (pseudo-deps filtered out)
    pub filename: Option<String>,
    // human metadata for the panel's mod browser: display name (jar-meta name ->
    // Modrinth title), Modrinth slug, author (jar-meta authorList -> team owner).
    pub name: Option<String>,
    pub author: Option<String>,
    pub slug: Option<String>,
    // Modrinth version id (from the sha1 lookup), so a build's Modrinth-sourced
    // mod can be re-added as a Modrinth source, not a non-existent cache jar.
    pub modrinth_version_id: Option<String>,
    // Release channel (release/beta/dev/unknown): from Modrinth version_type when
    // known, else unknown -- a jar carries no reliable channel of its own.
    pub channel: Option<String>,
    // Bytecode-derived facts (empty for a jar with no local bytes, e.g. a
    // Modrinth-only mod). owned_packages seed the package->owner index; the ref
    // sets become inferred requires/optional_dep edges once that index is built.
    pub owned_packages: Vec<String>,
    pub hard_refs: Vec<String>,
    pub optional_refs: Vec<String>,
    // Derived runtime side (both/client/server), or None when undecided.
    pub side: Option<String>,
}

pub struct BuildModSeed {
    pub sha1: String,
    pub filename: String,
    pub required: bool,
    pub default_enabled: bool,
}

pub struct PackSeed {
    pub pack_id: String,
    pub pack_version: String,
    pub mc_version: String,
    pub loader_id: Option<String>,
    pub loader_version: Option<String>,
    pub java_major: Option<i64>,
    pub fingerprint: Option<String>,
    pub mods: Vec<BuildModSeed>,
    pub conflicts: Vec<(String, String)>, // (a_sha1, b_sha1), from display.incompatible_with
}

#[derive(Default)]
pub struct ScanData {
    pub jars: Vec<JarSeed>,
    pub packs: Vec<PackSeed>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct HarvestReport {
    pub jars_scanned: usize,
    pub jars_no_identity: usize,
    pub mods: i64,
    pub mod_versions: i64,
    pub relations: i64,
    pub packs: usize,
    pub builds: i64,
    /// Bytecode-derived hard-dependency edges written this harvest.
    pub inferred_requires: i64,
    /// Bytecode-derived optional-dependency edges written this harvest.
    pub inferred_optional: i64,
    /// Jars whose client/server side was derived from the bytecode.
    pub sides_derived: i64,
}

// mcmod.info dependency lists routinely name the platform, not a real mod.
const PSEUDO_DEPS: &[&str] = &[
    "forge",
    "mcp",
    "minecraft",
    "fml",
    "cpw.mods.fml",
    "mod_mcversion",
];

/// Map a Modrinth `version_type` (release/beta/alpha) to a registry channel.
/// alpha collapses to beta (both pre-release); `dev` stays reserved for hand-set
/// developer builds. Unknown types yield None (release stays `unknown`).
fn channel_from_version_type(vt: &str) -> Option<String> {
    match vt {
        "release" => Some("release".to_string()),
        "beta" | "alpha" => Some("beta".to_string()),
        _ => None,
    }
}

fn filter_deps(deps: &[String]) -> Vec<String> {
    deps.iter()
        .filter(|d| {
            let l = d.trim().to_lowercase();
            !l.is_empty() && !PSEUDO_DEPS.contains(&l.as_str())
        })
        .map(|d| d.trim().to_string())
        .collect()
}

fn ae(e: crate::http::ApiError) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

/// Resolve a referenced package prefix to the modid of its owning mod, for an
/// inferred edge from `from_mod_id`. `None` when the prefix has no single owner,
/// its owner is the referencing mod itself, or the owner carries no modid.
fn resolve_edge_target(
    conn: &Connection,
    prefix: &str,
    from_mod_id: i64,
) -> Result<Option<String>> {
    let Some(owner) = queries::owner_mod_for_prefix(conn, prefix)? else {
        return Ok(None);
    };
    if owner == from_mod_id {
        return Ok(None);
    }
    queries::modid_for_mod(conn, owner)
}

/// Reconcile a scan into the registry, in one transaction. Pure (no I/O beyond
/// the connection); idempotent; never clobbers authored rows.
pub fn write_scan(conn: &Connection, scan: &ScanData, now: &str) -> Result<HarvestReport> {
    // sha1 -> modid, so curator conflict targets (selectors) can be expressed
    let modid_by_sha: HashMap<&str, &str> = scan
        .jars
        .iter()
        .filter_map(|j| j.modid.as_deref().map(|m| (j.sha1.as_str(), m)))
        .collect();

    // The bytecode-derived layer is purely rebuildable: wipe the package index
    // and the inferred edges up front, then re-derive from this scan. jar-meta
    // and authored/curator relations are a different source and untouched.
    conn.execute("DELETE FROM mod_package", [])?;
    conn.execute(
        "DELETE FROM relation WHERE source = 'inferred' AND kind IN ('requires', 'optional_dep')",
        [],
    )?;

    let mut sides_derived = 0i64;
    // (from_mod_id, jar) for jars carrying references, resolved to edges in a
    // second pass once every jar's packages are in the index.
    let mut derivations: Vec<(i64, &JarSeed)> = Vec::new();

    let mut no_identity = 0usize;
    for jar in &scan.jars {
        let mut aliases: Vec<(&str, &str)> = Vec::new();
        if let Some(m) = jar.modid.as_deref().filter(|s| !s.is_empty()) {
            aliases.push(("modid", m));
        }
        if let Some(p) = jar.project_id.as_deref().filter(|s| !s.is_empty()) {
            aliases.push(("modrinth", p));
        }
        if aliases.is_empty() {
            no_identity += 1;
            continue;
        }
        let mod_id = upsert::upsert_mod_by_alias(conn, &aliases, now)?;
        upsert::set_mod_meta(
            conn,
            mod_id,
            jar.name.as_deref(),
            jar.slug.as_deref(),
            jar.author.as_deref(),
            now,
        )?;
        // every loader the artifact suits; empty -> 'any' (handled downstream)
        let targets: Vec<&str> = jar.loaders.iter().map(String::as_str).collect();
        let version = jar.version.as_deref().unwrap_or("unknown");
        let mc_versions = if jar.mc_versions.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&jar.mc_versions)?)
        };
        upsert::upsert_mod_version(
            conn,
            mod_id,
            version,
            &targets,
            &jar.sha1,
            jar.size_bytes,
            jar.filename.as_deref(),
            mc_versions.as_deref(),
            now,
        )?;
        // after the row exists: record its Modrinth version id (by sha1)
        upsert::set_mod_version_modrinth(conn, &jar.sha1, jar.modrinth_version_id.as_deref(), now)?;
        // group the file under the release for its (version, channel)
        let channel = jar.channel.as_deref().unwrap_or("unknown");
        upsert::set_harvested_release(conn, &jar.sha1, mod_id, version, channel, now)?;
        for dep in &jar.requires {
            upsert::upsert_relation(
                conn,
                mod_id,
                dep,
                None,
                RelKind::Requires,
                Source::JarMeta,
                now,
            )?;
        }

        // bytecode-derived facts: this jar's owned packages (into the index) and
        // its side; its references are resolved to edges after the loop.
        let owned: Vec<&str> = jar.owned_packages.iter().map(String::as_str).collect();
        upsert::set_mod_packages(conn, mod_id, &owned)?;
        upsert::set_mod_version_side(conn, &jar.sha1, jar.side.as_deref(), now)?;
        if jar.side.is_some() {
            sides_derived += 1;
        }
        if !jar.hard_refs.is_empty() || !jar.optional_refs.is_empty() {
            derivations.push((mod_id, jar));
        }
    }

    // Second pass: resolve each referenced package prefix to its owning mod and
    // record an inferred edge. A prefix with no single owner (unknown or shaded)
    // is skipped; a hard edge to a target suppresses a competing optional one.
    let mut inferred_requires = 0i64;
    let mut inferred_optional = 0i64;
    for (from_mod_id, jar) in &derivations {
        let mut hard_targets: HashSet<String> = HashSet::new();
        for prefix in &jar.hard_refs {
            if let Some(target) = resolve_edge_target(conn, prefix, *from_mod_id)?
                && hard_targets.insert(target.clone())
            {
                upsert::upsert_relation(
                    conn,
                    *from_mod_id,
                    &target,
                    None,
                    RelKind::Requires,
                    Source::Inferred,
                    now,
                )?;
                inferred_requires += 1;
            }
        }
        let mut opt_targets: HashSet<String> = HashSet::new();
        for prefix in &jar.optional_refs {
            if let Some(target) = resolve_edge_target(conn, prefix, *from_mod_id)?
                && !hard_targets.contains(&target)
                && opt_targets.insert(target.clone())
            {
                upsert::upsert_relation(
                    conn,
                    *from_mod_id,
                    &target,
                    None,
                    RelKind::OptionalDep,
                    Source::Inferred,
                    now,
                )?;
                inferred_optional += 1;
            }
        }
    }

    for pack in &scan.packs {
        upsert::upsert_pack(conn, &pack.pack_id, now)?;
        let build = upsert::upsert_pack_build(
            conn,
            &pack.pack_id,
            &pack.pack_version,
            &pack.mc_version,
            pack.loader_id.as_deref(),
            pack.loader_version.as_deref(),
            pack.java_major,
            pack.fingerprint.as_deref(),
            true,
            now,
        )?;
        for bm in &pack.mods {
            if let Some(mv) = upsert::mod_version_id_for_sha1(conn, &bm.sha1)? {
                upsert::link_build_mod(
                    conn,
                    build,
                    mv,
                    &bm.filename,
                    bm.required,
                    bm.default_enabled,
                )?;
            }
        }
        // curator conflicts: A's mod_id conflicts with B's modid (and reverse)
        for (a_sha, b_sha) in &pack.conflicts {
            if let (Some(a_mod), Some(b_modid)) = (
                queries::mod_id_for_sha1(conn, a_sha)?,
                modid_by_sha.get(b_sha.as_str()),
            ) {
                upsert::upsert_relation(
                    conn,
                    a_mod,
                    b_modid,
                    None,
                    RelKind::Conflicts,
                    Source::Curator,
                    now,
                )?;
            }
            if let (Some(b_mod), Some(a_modid)) = (
                queries::mod_id_for_sha1(conn, b_sha)?,
                modid_by_sha.get(a_sha.as_str()),
            ) {
                upsert::upsert_relation(
                    conn,
                    b_mod,
                    a_modid,
                    None,
                    RelKind::Conflicts,
                    Source::Curator,
                    now,
                )?;
            }
        }
    }

    // drop the provisional 'unknown' releases left empty once files moved to
    // their channel / content-signature release
    upsert::prune_empty_releases(conn)?;

    let s = queries::stats(conn)?;
    Ok(HarvestReport {
        jars_scanned: scan.jars.len(),
        jars_no_identity: no_identity,
        mods: s.mods,
        mod_versions: s.mod_versions,
        relations: s.relations,
        packs: scan.packs.len(),
        builds: s.builds,
        inferred_requires,
        inferred_optional,
        sides_derived,
    })
}

/// Scan the storage tree + Modrinth into a [ScanData]. Async (FS reads + one
/// batched Modrinth lookup); does not touch the registry.
pub async fn scan(storage: &Storage, modrinth: &Modrinth) -> Result<ScanData> {
    let inventory = storage.list_cache_inventory().await.map_err(ae)?;
    let mut size_by_sha: HashMap<String, i64> = inventory
        .iter()
        .map(|e| (e.sha1.clone(), e.size_bytes as i64))
        .collect();
    let mut all_shas: HashSet<String> = inventory.iter().map(|e| e.sha1.clone()).collect();

    // read mcmod.info + derive jar facts (loader marker, content signature) +
    // scan bytecode (owned packages, references, side) from every cached jar
    // (Modrinth-only mods have no local jar to read)
    let mut mcmod_by_sha: HashMap<String, McModInfo> = HashMap::new();
    let mut facts_by_sha: HashMap<String, JarFacts> = HashMap::new();
    let mut bytecode_by_sha: HashMap<String, bytecode::JarBytecode> = HashMap::new();
    for e in &inventory {
        let Ok(path) = storage.cache_jar_path(&e.sha1[..2], &e.sha1) else {
            continue;
        };
        let Ok(bytes) = tokio::fs::read(&path).await else {
            continue;
        };
        facts_by_sha.insert(e.sha1.clone(), jar_facts(&bytes));
        bytecode_by_sha.insert(e.sha1.clone(), bytecode::scan_jar(&bytes));
        if let Ok(Some(info)) = read_mcmod_info(&bytes)
            && !info.modid.is_empty()
        {
            mcmod_by_sha.insert(e.sha1.clone(), info);
        }
    }

    // published builds + curator conflicts, per pack
    let mut packs = Vec::new();
    let mut filename_by_sha: HashMap<String, String> = HashMap::new();
    for pid in storage.list_authoring_packs().await.map_err(ae)? {
        let Ok(manifest) = storage.load_latest_manifest(&pid).await else {
            continue; // unbuilt pack -> no published build to record
        };
        let mut mods = Vec::new();
        let mut sha_by_filename: HashMap<String, String> = HashMap::new();
        for m in &manifest.mods {
            all_shas.insert(m.sha1.clone());
            size_by_sha
                .entry(m.sha1.clone())
                .or_insert(m.size_bytes as i64);
            filename_by_sha
                .entry(m.sha1.clone())
                .or_insert_with(|| m.filename.clone());
            sha_by_filename.insert(m.filename.clone(), m.sha1.clone());
            mods.push(BuildModSeed {
                sha1: m.sha1.clone(),
                filename: m.filename.clone(),
                required: m.required,
                default_enabled: m.default_enabled,
            });
        }
        // conflicts come from each mod's published display.incompatible_with
        // (set per-mod in the config now, not a curator table). sha_by_filename is
        // complete from the loop above, so a target naming a later mod resolves.
        let mut conflicts = Vec::new();
        for m in &manifest.mods {
            let Some(disp) = &m.display else { continue };
            for b_fname in &disp.incompatible_with {
                if let Some(b_sha) = sha_by_filename.get(b_fname) {
                    conflicts.push((m.sha1.clone(), b_sha.clone()));
                }
            }
        }
        packs.push(PackSeed {
            pack_id: pid,
            pack_version: manifest.pack_version.clone(),
            mc_version: manifest.minecraft.version.clone(),
            loader_id: Some(manifest.loader.name.clone()),
            loader_version: Some(manifest.loader.version.clone()),
            java_major: Some(manifest.java.major as i64),
            fingerprint: manifest.fingerprint.clone(),
            mods,
            conflicts,
        });
    }

    // one batched identity lookup over every sha1 we know
    let sha_vec: Vec<String> = all_shas.iter().cloned().collect();
    let modrinth_by_sha = match modrinth.version_files_by_sha1(&sha_vec).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "modrinth lookup failed; harvesting jar-meta only");
            HashMap::new()
        }
    };

    // enrich human metadata (name/slug/author) for Modrinth-identified jars: one
    // batched project lookup for title+slug+team, then one batched team lookup for
    // the owner username. Both degrade to empty on failure -- jar-meta still fills
    // name/author where present, identity harvest is unaffected.
    //
    // Privacy: only hit Modrinth for a sha whose local jar-meta is missing a name
    // or author. A jar that already names itself + its authors needs no project or
    // team call -- the egress stays limited to what the mirror cannot answer from
    // the bytes it already holds (a slug, only Modrinth has, is the tradeoff).
    let needs_enrich = |sha: &str| match mcmod_by_sha.get(sha) {
        Some(i) => i.name.trim().is_empty() || i.authors.is_empty(),
        None => true,
    };
    let project_ids: Vec<String> = modrinth_by_sha
        .iter()
        .filter(|(sha, _)| needs_enrich(sha))
        .map(|(_, v)| v.project_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let projects = match modrinth.projects_by_ids(&project_ids).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "modrinth project lookup failed; names/slugs from jar-meta only");
            HashMap::new()
        }
    };
    let team_ids: Vec<String> = projects
        .values()
        .map(|p| p.team.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let team_owners = match modrinth.team_owners_by_ids(&team_ids).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "modrinth team lookup failed; authors from jar-meta only");
            HashMap::new()
        }
    };

    let jars = all_shas
        .into_iter()
        .map(|sha| {
            let info = mcmod_by_sha.get(&sha);
            let mrv = modrinth_by_sha.get(&sha);
            let facts = facts_by_sha.get(&sha);
            let bc = bytecode_by_sha.get(&sha);
            let project = mrv.and_then(|v| projects.get(&v.project_id));
            // name: jar-meta name wins (local), else Modrinth title
            let name = info
                .map(|i| i.name.clone())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    project
                        .map(|p| p.title.clone())
                        .filter(|s| !s.trim().is_empty())
                });
            // author: jar-meta authorList wins (local), else the project's team owner
            let author = info
                .map(|i| i.authors.join(", "))
                .filter(|s| !s.trim().is_empty())
                .or_else(|| project.and_then(|p| team_owners.get(&p.team).cloned()));
            // slug is a Modrinth concept; jars carry none
            let slug = project.map(|p| p.slug.clone()).filter(|s| !s.is_empty());
            JarSeed {
                size_bytes: size_by_sha.get(&sha).copied().unwrap_or(0),
                modid: info.map(|i| i.modid.clone()).filter(|s| !s.is_empty()),
                version: info
                    .map(|i| i.version.clone())
                    .filter(|s| !s.is_empty())
                    .or_else(|| mrv.map(|v| v.version_number.clone())),
                project_id: mrv.map(|v| v.project_id.clone()),
                // loader: Modrinth's set wins; else the jar's own marker
                // (mcmod.info/mods.toml -> forge, fabric.mod.json -> fabric); else
                // empty (-> 'any' downstream)
                loaders: match mrv.map(|v| v.loaders.clone()).filter(|l| !l.is_empty()) {
                    Some(l) => l,
                    None => facts.and_then(|f| f.loader.clone()).into_iter().collect(),
                },
                // mc: Modrinth's set wins; else the jar's declared mcversion when
                // it looks like a real version (not a gradle token)
                mc_versions: match mrv
                    .map(|v| v.game_versions.clone())
                    .filter(|g| !g.is_empty())
                {
                    Some(g) => g,
                    None => info
                        .and_then(|i| clean_mc_version(&i.mcversion))
                        .into_iter()
                        .collect(),
                },
                requires: info
                    .map(|i| filter_deps(&i.dependencies))
                    .unwrap_or_default(),
                filename: filename_by_sha.get(&sha).cloned(),
                name,
                author,
                slug,
                modrinth_version_id: mrv.map(|v| v.id.clone()),
                channel: mrv.and_then(|v| channel_from_version_type(&v.version_type)),
                owned_packages: bc
                    .map(|b| b.owned.iter().cloned().collect())
                    .unwrap_or_default(),
                hard_refs: bc
                    .map(|b| b.hard_refs.iter().cloned().collect())
                    .unwrap_or_default(),
                optional_refs: bc
                    .map(|b| b.optional_refs.iter().cloned().collect())
                    .unwrap_or_default(),
                side: bc.and_then(|b| b.side).map(|s| s.as_str().to_string()),
                sha1: sha,
            }
        })
        .collect();

    Ok(ScanData { jars, packs })
}

/// Full harvest: scan (async) then write (in a blocking transaction).
pub async fn run_harvest(
    storage: &Storage,
    modrinth: &Modrinth,
    registry: Arc<Registry>,
) -> Result<HarvestReport> {
    let scan = scan(storage, modrinth).await?;
    let now = upsert::now_rfc3339();
    let report =
        tokio::task::spawn_blocking(move || registry.with_txn(|c| write_scan(c, &scan, &now)))
            .await
            .map_err(|e| anyhow::anyhow!("harvest write task: {e}"))??;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ScanData {
        ScanData {
            jars: vec![
                JarSeed {
                    sha1: "sha_a".into(),
                    size_bytes: 100,
                    channel: None,
                    modid: Some("appleskin".into()),
                    version: Some("2.5".into()),
                    project_id: Some("EsAfb37o".into()),
                    loaders: vec!["forge".into()],
                    mc_versions: vec!["1.12.2".into()],
                    requires: vec![],
                    filename: Some("appleskin.jar".into()),
                    name: Some("AppleSkin".into()),
                    author: Some("squeek502".into()),
                    slug: Some("appleskin".into()),
                    modrinth_version_id: Some("mrv_apple".into()),
                    owned_packages: vec![],
                    hard_refs: vec![],
                    optional_refs: vec![],
                    side: None,
                },
                JarSeed {
                    sha1: "sha_b".into(),
                    size_bytes: 200,
                    channel: None,
                    modid: Some("jei".into()),
                    version: Some("4.16".into()),
                    project_id: None,
                    loaders: vec!["forge".into()],
                    mc_versions: vec![],
                    requires: vec!["appleskin".into(), "forge".into()], // 'forge' filtered upstream
                    filename: Some("jei.jar".into()),
                    name: None,
                    author: None,
                    slug: None,
                    modrinth_version_id: None,
                    owned_packages: vec![],
                    hard_refs: vec![],
                    optional_refs: vec![],
                    side: None,
                },
                JarSeed {
                    sha1: "sha_noid".into(),
                    size_bytes: 50,
                    channel: None,
                    modid: None,
                    version: None,
                    project_id: None,
                    loaders: vec![],
                    mc_versions: vec![],
                    requires: vec![],
                    filename: Some("mystery.jar".into()),
                    name: None,
                    author: None,
                    slug: None,
                    modrinth_version_id: None,
                    owned_packages: vec![],
                    hard_refs: vec![],
                    optional_refs: vec![],
                    side: None,
                },
            ],
            packs: vec![PackSeed {
                pack_id: "Industrial".into(),
                pack_version: "2026.06.06".into(),
                mc_version: "1.12.2".into(),
                loader_id: Some("forge".into()),
                loader_version: Some("14.23".into()),
                java_major: Some(8),
                fingerprint: Some("fp_test".into()),
                mods: vec![
                    BuildModSeed {
                        sha1: "sha_a".into(),
                        filename: "appleskin.jar".into(),
                        required: true,
                        default_enabled: true,
                    },
                    BuildModSeed {
                        sha1: "sha_b".into(),
                        filename: "jei.jar".into(),
                        required: true,
                        default_enabled: true,
                    },
                ],
                conflicts: vec![("sha_a".into(), "sha_b".into())],
            }],
        }
    }

    #[test]
    fn write_scan_harvests_idempotently_and_preserves_authored() {
        let r = Registry::open_in_memory().unwrap();
        let scan = sample();

        let rep = r.with_txn(|c| write_scan(c, &scan, "T0")).unwrap();
        assert_eq!(
            rep.jars_no_identity, 1,
            "the no-modid no-modrinth jar is skipped"
        );
        assert_eq!(rep.mod_versions, 2);

        r.with_conn(|c| {
            // modid + project_id collapse to one identity
            let by_modid = queries::mod_id_for_alias(c, "modid", "appleskin")?;
            let by_proj = queries::mod_id_for_alias(c, "modrinth", "EsAfb37o")?;
            assert!(by_modid.is_some() && by_modid == by_proj);
            // write_scan stores requires verbatim; pseudo-dep filtering happens
            // in scan() (covered by filter_deps_drops_platform_pseudo_deps), so
            // the sample's ["appleskin","forge"] both land here.
            let req: i64 = c.query_row(
                "SELECT count(*) FROM relation WHERE kind='requires'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(req, 2);
            // conflict recorded both directions
            let conf: i64 = c.query_row(
                "SELECT count(*) FROM relation WHERE kind='conflicts'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(conf, 2);
            // the build's content fingerprint lands in pack_build
            let fp: Option<String> = c.query_row(
                "SELECT fingerprint FROM pack_build WHERE pack_id='Industrial'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(fp.as_deref(), Some("fp_test"));
            // human metadata from the seed lands on the mods row
            let apple = queries::mod_id_for_alias(c, "modid", "appleskin")?.unwrap();
            let (name, author, slug): (Option<String>, Option<String>, Option<String>) = c
                .query_row(
                    "SELECT canonical_name, author, slug FROM mods WHERE id = ?1",
                    [apple],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
            assert_eq!(name.as_deref(), Some("AppleSkin"));
            assert_eq!(author.as_deref(), Some("squeek502"));
            assert_eq!(slug.as_deref(), Some("appleskin"));
            let mrv: Option<String> = c.query_row(
                "SELECT modrinth_version_id FROM mod_version WHERE sha1 = 'sha_a'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(mrv.as_deref(), Some("mrv_apple"));
            Ok(())
        })
        .unwrap();

        // idempotent re-run
        let rep2 = r.with_txn(|c| write_scan(c, &scan, "T1")).unwrap();
        assert_eq!(rep2.mods, rep.mods);
        assert_eq!(rep2.mod_versions, rep.mod_versions);
        assert_eq!(rep2.relations, rep.relations);
        assert_eq!(rep2.builds, rep.builds);

        // promote a row to authored, re-harvest, confirm it survives untouched
        r.with_txn(|c| {
            c.execute(
                "UPDATE mod_version SET source='authored', version='AUTH' WHERE sha1='sha_b'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        r.with_txn(|c| write_scan(c, &scan, "T2")).unwrap();
        r.with_conn(|c| {
            let v: String = c.query_row(
                "SELECT version FROM mod_version WHERE sha1='sha_b'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(v, "AUTH");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn filter_deps_drops_platform_pseudo_deps() {
        let got = filter_deps(&[
            "appleskin".into(),
            "Forge".into(),
            "minecraft".into(),
            "  ".into(),
            "jei".into(),
        ]);
        assert_eq!(got, vec!["appleskin".to_string(), "jei".to_string()]);
    }

    fn jar(sha: &str, modid: &str, version: Option<&str>, loaders: Vec<String>) -> JarSeed {
        JarSeed {
            sha1: sha.into(),
            size_bytes: 1,
            channel: None,
            modid: Some(modid.into()),
            version: version.map(Into::into),
            project_id: None,
            loaders,
            mc_versions: vec![],
            requires: vec![],
            filename: Some(format!("{sha}.jar")),
            name: None,
            author: None,
            slug: None,
            modrinth_version_id: None,
            owned_packages: vec![],
            hard_refs: vec![],
            optional_refs: vec![],
            side: None,
        }
    }

    /// A jar seed carrying bytecode-derived facts, for the derivation tests.
    fn dseed(
        sha: &str,
        modid: &str,
        owned: &[&str],
        hard: &[&str],
        opt: &[&str],
        side: Option<&str>,
    ) -> JarSeed {
        let strs = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        JarSeed {
            sha1: sha.into(),
            size_bytes: 1,
            channel: None,
            modid: Some(modid.into()),
            version: Some("1".into()),
            project_id: None,
            loaders: vec!["forge".into()],
            mc_versions: vec![],
            requires: vec![],
            filename: Some(format!("{sha}.jar")),
            name: None,
            author: None,
            slug: None,
            modrinth_version_id: None,
            owned_packages: strs(owned),
            hard_refs: strs(hard),
            optional_refs: strs(opt),
            side: side.map(String::from),
        }
    }

    // The core of #40: owned packages populate the index, references resolve to
    // inferred requires/optional_dep edges, a jar's own reference makes no
    // self-edge, and the derived side lands on the artifact.
    #[test]
    fn write_scan_derives_inferred_edges_and_side() {
        let r = Registry::open_in_memory().unwrap();
        let scan = ScanData {
            jars: vec![
                dseed(
                    "sha_ae2",
                    "appliedenergistics2",
                    &["appeng/api", "appeng/core"],
                    &["appeng/api"], // own package -> must NOT self-edge
                    &[],
                    Some("both"),
                ),
                dseed(
                    "sha_stuff",
                    "ae2stuff",
                    &["ae2stuff/block"],
                    &["appeng/api"],
                    &[],
                    None,
                ),
                dseed("sha_jei", "jei", &["mezz/jei"], &[], &[], None),
                dseed(
                    "sha_jeibees",
                    "jeibees",
                    &["jeibees/core"],
                    &[],
                    &["mezz/jei"],
                    None,
                ),
            ],
            packs: vec![],
        };

        let rep = r.with_txn(|c| write_scan(c, &scan, "T0")).unwrap();
        assert_eq!(
            rep.inferred_requires, 1,
            "ae2stuff -> AE2 (AE2's self-ref skipped)"
        );
        assert_eq!(rep.inferred_optional, 1, "jeibees -> JEI (optional)");
        assert_eq!(rep.sides_derived, 1);

        r.with_conn(|c| {
            let stuff = queries::mod_id_for_alias(c, "modid", "ae2stuff")?.unwrap();
            let (target, kind, source): (String, String, String) = c.query_row(
                "SELECT target_modid, kind, source FROM relation WHERE from_mod_id = ?1",
                [stuff],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!(
                (target.as_str(), kind.as_str(), source.as_str()),
                ("appliedenergistics2", "requires", "inferred")
            );

            let jeibees = queries::mod_id_for_alias(c, "modid", "jeibees")?.unwrap();
            let (t2, k2): (String, String) = c.query_row(
                "SELECT target_modid, kind FROM relation WHERE from_mod_id = ?1",
                [jeibees],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!((t2.as_str(), k2.as_str()), ("jei", "optional_dep"));

            let ae2 = queries::mod_id_for_alias(c, "modid", "appliedenergistics2")?.unwrap();
            let self_edges: i64 = c.query_row(
                "SELECT count(*) FROM relation WHERE from_mod_id = ?1",
                [ae2],
                |r| r.get(0),
            )?;
            assert_eq!(
                self_edges, 0,
                "no self-dependency from an own-package reference"
            );

            let side: Option<String> = c.query_row(
                "SELECT side FROM mod_version WHERE sha1 = 'sha_ae2'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(side.as_deref(), Some("both"));
            assert_eq!(queries::owner_mod_for_prefix(c, "appeng/api")?, Some(ae2));
            Ok(())
        })
        .unwrap();

        // idempotent: inferred edges are wiped and rebuilt, never duplicated
        let rep2 = r.with_txn(|c| write_scan(c, &scan, "T1")).unwrap();
        assert_eq!(rep2.inferred_requires, 1);
        assert_eq!(rep2.inferred_optional, 1);
        r.with_conn(|c| {
            let total: i64 = c.query_row(
                "SELECT count(*) FROM relation WHERE source = 'inferred'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(total, 2, "no duplicate inferred edges after re-harvest");
            Ok(())
        })
        .unwrap();
    }

    // A prefix owned by two mods is an ambiguous shaded library: it resolves to no
    // single owner, so it yields no edge.
    #[test]
    fn write_scan_skips_ambiguous_shaded_prefix() {
        let r = Registry::open_in_memory().unwrap();
        let scan = ScanData {
            jars: vec![
                dseed(
                    "sha_a",
                    "moda",
                    &["org/shaded", "moda/core"],
                    &[],
                    &[],
                    None,
                ),
                dseed(
                    "sha_b",
                    "modb",
                    &["org/shaded", "modb/core"],
                    &[],
                    &[],
                    None,
                ),
                dseed("sha_c", "modc", &["modc/core"], &["org/shaded"], &[], None),
            ],
            packs: vec![],
        };
        let rep = r.with_txn(|c| write_scan(c, &scan, "T0")).unwrap();
        assert_eq!(rep.inferred_requires, 0, "ambiguous prefix -> no edge");
    }

    // #1: two distinct jars of one mod with no version metadata both become
    // version='unknown'; the old UNIQUE(mod_id, version, target) crashed the
    // harvest on the second. sha1 is the only identity now.
    #[test]
    fn write_scan_allows_two_unversioned_jars_of_one_mod() {
        let r = Registry::open_in_memory().unwrap();
        let scan = ScanData {
            jars: vec![
                jar("sha_x", "dup", None, vec!["forge".into()]),
                jar("sha_y", "dup", None, vec!["forge".into()]),
            ],
            packs: vec![],
        };
        let rep = r.with_txn(|c| write_scan(c, &scan, "T0")).unwrap();
        assert_eq!(rep.mods, 1, "same modid -> one identity");
        assert_eq!(rep.mod_versions, 2, "two distinct jars are two artifacts");
        r.with_conn(|c| {
            // both reachable as two versions of the one mod
            assert_eq!(queries::versions_of_mod(c, "modid", "dup")?.len(), 2);
            Ok(())
        })
        .unwrap();
    }

    // #2: a jar published for several loaders records every target; an empty
    // loader set falls back to 'any'; a re-harvest replaces the set, not appends.
    #[test]
    fn write_scan_records_target_set_with_any_fallback_and_replace() {
        let r = Registry::open_in_memory().unwrap();
        let scan = ScanData {
            jars: vec![
                jar(
                    "sha_multi",
                    "multi",
                    Some("1"),
                    vec!["forge".into(), "fabric".into()],
                ),
                jar("sha_any", "tweak", Some("1"), vec![]),
            ],
            packs: vec![],
        };
        r.with_txn(|c| write_scan(c, &scan, "T0")).unwrap();
        r.with_conn(|c| {
            let mut multi = queries::versions_of_mod(c, "modid", "multi")?[0]
                .targets
                .clone();
            multi.sort();
            assert_eq!(multi, vec!["fabric".to_string(), "forge".to_string()]);
            let tweak = queries::versions_of_mod(c, "modid", "tweak")?;
            assert_eq!(tweak[0].targets, vec!["any".to_string()], "empty -> any");
            Ok(())
        })
        .unwrap();

        // upstream dropped fabric support: the set shrinks, doesn't accumulate
        let scan2 = ScanData {
            jars: vec![jar("sha_multi", "multi", Some("1"), vec!["forge".into()])],
            packs: vec![],
        };
        r.with_txn(|c| write_scan(c, &scan2, "T1")).unwrap();
        r.with_conn(|c| {
            let multi = queries::versions_of_mod(c, "modid", "multi")?;
            assert_eq!(multi[0].targets, vec!["forge".to_string()], "set replaced");
            Ok(())
        })
        .unwrap();
    }

    // an authored artifact keeps its hand-set targets even when a re-harvest
    // reports a different loader set (the precious skip in set_mod_version_targets)
    #[test]
    fn write_scan_preserves_authored_targets() {
        let r = Registry::open_in_memory().unwrap();
        let scan = ScanData {
            jars: vec![jar(
                "sha_p",
                "pmod",
                Some("1"),
                vec!["forge".into(), "fabric".into()],
            )],
            packs: vec![],
        };
        r.with_txn(|c| write_scan(c, &scan, "T0")).unwrap();
        r.with_txn(|c| {
            c.execute(
                "UPDATE mod_version SET source = 'authored' WHERE sha1 = 'sha_p'",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        // re-harvest now sees only forge upstream; the authored targets must hold
        let scan2 = ScanData {
            jars: vec![jar("sha_p", "pmod", Some("1"), vec!["forge".into()])],
            packs: vec![],
        };
        r.with_txn(|c| write_scan(c, &scan2, "T1")).unwrap();
        r.with_conn(|c| {
            let mut t = queries::versions_of_mod(c, "modid", "pmod")?[0]
                .targets
                .clone();
            t.sort();
            assert_eq!(
                t,
                vec!["fabric".to_string(), "forge".to_string()],
                "authored targets survive re-harvest"
            );
            Ok(())
        })
        .unwrap();
    }

    // A Modrinth channel lands on the release, and the provisional 'unknown'
    // release upsert_mod_version created is pruned once the file moves.
    #[test]
    fn write_scan_applies_channel_and_prunes_empty_release() {
        let r = Registry::open_in_memory().unwrap();
        let mut a = jar("sha_beta", "betamod", Some("0.9"), vec!["forge".into()]);
        a.channel = Some("beta".into());
        let scan = ScanData {
            jars: vec![a],
            packs: vec![],
        };
        r.with_txn(|c| write_scan(c, &scan, "T0")).unwrap();
        r.with_conn(|c| {
            let mid = queries::mod_id_for_alias(c, "modid", "betamod")?.unwrap();
            let rels = queries::releases_of_mod_by_id(c, mid)?;
            assert_eq!(rels.len(), 1);
            assert_eq!(rels[0].channel, "beta");
            let total: i64 = c.query_row("SELECT count(*) FROM mod_release", [], |r| r.get(0))?;
            assert_eq!(total, 1, "no empty leftover release");
            Ok(())
        })
        .unwrap();
    }
}
