//! Registry harvest: scan the cache + published manifests, read each jar's
//! `mcmod.info`, batch-resolve Modrinth identity, and reconcile it all into the
//! registry. The scan (FS + network) is async; the write (`write_scan`) is a
//! pure, transactional DB step that's unit-tested without I/O.
//!
//! Phase 1 is harvest-only: it writes `source = harvested | jar-meta | curator`
//! rows and never clobbers authored ones. Modrinth project-level deps are not
//! harvested yet (their target is a project_id, a different selector namespace
//! than the modid relations here) -- that lands with the Phase 4 resolver.

use super::curator::{McModInfo, parse_curator, read_mcmod_info};
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
}

pub struct BuildModSeed {
    pub sha1: String,
    pub filename: String,
    pub required: bool,
    pub default_enabled: bool,
}

pub struct PackSeed {
    pub pack_id: String,
    pub provenance: String,
    pub pack_version: String,
    pub mc_version: String,
    pub loader_id: Option<String>,
    pub loader_version: Option<String>,
    pub java_major: Option<i64>,
    pub fingerprint: Option<String>,
    pub mods: Vec<BuildModSeed>,
    pub conflicts: Vec<(String, String)>, // (a_sha1, b_sha1), from curator.incompatible
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

/// Reconcile a scan into the registry, in one transaction. Pure (no I/O beyond
/// the connection); idempotent; never clobbers authored rows.
pub fn write_scan(conn: &Connection, scan: &ScanData, now: &str) -> Result<HarvestReport> {
    // sha1 -> modid, so curator conflict targets (selectors) can be expressed
    let modid_by_sha: HashMap<&str, &str> = scan
        .jars
        .iter()
        .filter_map(|j| j.modid.as_deref().map(|m| (j.sha1.as_str(), m)))
        .collect();

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
    }

    for pack in &scan.packs {
        upsert::upsert_pack(conn, &pack.pack_id, &pack.provenance, now)?;
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

    let s = queries::stats(conn)?;
    Ok(HarvestReport {
        jars_scanned: scan.jars.len(),
        jars_no_identity: no_identity,
        mods: s.mods,
        mod_versions: s.mod_versions,
        relations: s.relations,
        packs: scan.packs.len(),
        builds: s.builds,
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

    // read mcmod.info from every cached jar (Modrinth-only mods have no local jar)
    let mut mcmod_by_sha: HashMap<String, McModInfo> = HashMap::new();
    for e in &inventory {
        let Ok(path) = storage.cache_jar_path(&e.sha1[..2], &e.sha1) else {
            continue;
        };
        let Ok(bytes) = tokio::fs::read(&path).await else {
            continue;
        };
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
        let mut conflicts = Vec::new();
        if let Ok(text) = storage.load_curator_doc(&pid).await
            && let Ok(cur) = parse_curator(&text)
        {
            for (a_fname, blist) in &cur.incompatible {
                if let Some(a_sha) = sha_by_filename.get(a_fname) {
                    for b_fname in blist {
                        if let Some(b_sha) = sha_by_filename.get(b_fname) {
                            conflicts.push((a_sha.clone(), b_sha.clone()));
                        }
                    }
                }
            }
        }
        packs.push(PackSeed {
            pack_id: pid,
            provenance: "hivens".into(), // Phase 1 default; operator sets it in Phase 2
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
            let project = mrv.and_then(|v| projects.get(&v.project_id));
            // name: jar-meta name wins (local), else Modrinth title
            let name = info
                .map(|i| i.name.clone())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| project.map(|p| p.title.clone()).filter(|s| !s.trim().is_empty()));
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
                loaders: mrv.map(|v| v.loaders.clone()).unwrap_or_default(),
                mc_versions: mrv.map(|v| v.game_versions.clone()).unwrap_or_default(),
                requires: info
                    .map(|i| filter_deps(&i.dependencies))
                    .unwrap_or_default(),
                filename: filename_by_sha.get(&sha).cloned(),
                name,
                author,
                slug,
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
                },
                JarSeed {
                    sha1: "sha_b".into(),
                    size_bytes: 200,
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
                },
                JarSeed {
                    sha1: "sha_noid".into(),
                    size_bytes: 50,
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
                },
            ],
            packs: vec![PackSeed {
                pack_id: "Industrial".into(),
                provenance: "sc".into(),
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
        }
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
}
