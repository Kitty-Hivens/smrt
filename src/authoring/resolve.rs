//! The resolve pass: read the registry dependency graph for a pack's declared
//! mods and report the problems a build would otherwise only surface at crash
//! time -- an unmet hard dependency, an active conflict, a same-capability
//! overlap, or a present dependency whose version falls outside the window a
//! requirer declared. It also hints which declared mods are depended-on (so they
//! should stay required).
//!
//! Pure over a `&Connection` (the handler runs it inside `spawn_blocking` via
//! `Registry::with_conn`). It never mutates the config: required/optional stays
//! the pack's own decision, and any override of a derived edge is debug-gated
//! elsewhere. When it cannot decide something confidently -- a mod with no
//! registry identity, a version string it cannot compare against a window -- it
//! reports that as unresolved/unchecked rather than guess, so a flagged problem
//! is a real one.
//!
//! Edges are read at artifact granularity (#48): a pack declares a file, so the
//! facts that apply are the ones that file declares, plus whatever is asserted
//! about its mod as a whole. A jar the registry has never read gets the mod-level
//! facts only -- it does not borrow a sibling version's dependencies.

use crate::domain::{PackConfig, SourceDecl};
use crate::registry::model::{GraphData, GraphEdge, RelKind};
use crate::registry::{queries, semver};
use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use ts_rs::TS;

/// The outcome of resolving a pack against the registry graph. Arrays are empty
/// when clean; `missing` and `conflicts` are the blocking problems, the rest are
/// advisory. All lists are sorted for a stable render.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ResolveReport {
    #[ts(type = "number")]
    pub declared_mods: usize,
    /// How many declared jar mods are identified: a registry identity, or a valid
    /// Modrinth pin the mirror has not harvested yet (present at build). The rest
    /// are in `unresolved`.
    #[ts(type = "number")]
    pub resolved_mods: usize,
    /// A hard dependency no present mod satisfies -- the pack would crash.
    pub missing: Vec<MissingDep>,
    /// Two present mods the graph says cannot run together, both in the default
    /// install -- a live conflict the pack ships with.
    pub conflicts: Vec<ActiveConflict>,
    /// The same incompatibility, but with at least one side an opted-out optional
    /// -- it only bites if the user turns that mod on, so it is advisory, not a
    /// blocking problem (#9).
    pub optional_conflicts: Vec<ActiveConflict>,
    /// A capability more than one present mod provides -- usually redundant, and
    /// the two may fight over the same hook.
    pub overlaps: Vec<CapabilityOverlap>,
    /// A present dependency whose shipped version sits outside a requirer's
    /// declared window.
    pub version_issues: Vec<VersionIssue>,
    /// A present mod that another present mod requires but that the pack marks
    /// optional -- it should be required.
    pub required_hints: Vec<RequiredHint>,
    /// Declared artifacts this pack's loader cannot run, with nothing present to
    /// bridge them -- they will not load at all (#50).
    pub loader_mismatch: Vec<LoaderMismatch>,
    /// Foreign-loader artifacts a present connector carries. Not a problem: they
    /// load. Listed because it is worth knowing which mods in a forge pack are
    /// actually fabric mods riding the bridge -- pull the connector and they all
    /// go at once.
    pub loader_bridged: Vec<LoaderMismatch>,
    /// Declared jar mods with no identity and no valid pin: an un-harvested
    /// `smrt_cache` upload the mirror has not read. Left unjudged, listed so the
    /// operator knows coverage was partial. A Modrinth pin is not here -- it is a
    /// valid declaration, counted in `resolved_mods`.
    pub unresolved: Vec<String>,
    /// How many version windows could not be checked because a version string was
    /// not plainly comparable (a classifier suffix, an embedded MC prefix).
    #[ts(type = "number")]
    pub version_windows_unchecked: usize,
}

/// A required target no present mod satisfies. `target` is the graph selector
/// (a modid, or `modrinth:<project_id>`); `needed_by` are the filenames that
/// require it; `source` is the provenance of the authoritative edge.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct MissingDep {
    pub target: String,
    pub needed_by: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version_range: Option<String>,
    pub source: String,
}

/// Two present mods the graph marks incompatible. `breaks` distinguishes the
/// harder `breaks` kind from a plain `conflicts`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ActiveConflict {
    pub a: String,
    pub b: String,
    pub breaks: bool,
    pub source: String,
}

/// A capability more than one present mod provides.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct CapabilityOverlap {
    pub capability: String,
    pub mods: Vec<String>,
}

/// A present dependency shipping a version outside a declared window.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct VersionIssue {
    pub target: String,
    pub filename: String,
    pub present_version: String,
    pub required_range: String,
    pub needed_by: Vec<String>,
}

/// A declared artifact built for loaders this pack does not run.
///
/// A pack natively runs its own loader and everything that loader inherits from
/// (cleanroom runs forge's artifacts, quilt runs fabric's). Anything else needs a
/// bridge -- a connector mod that carries another loader's mods at runtime, which
/// the registry records as a `provides` of the `loader:<name>` capability.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct LoaderMismatch {
    pub filename: String,
    /// the loaders the artifact was actually published for
    pub artifact_loaders: Vec<String>,
    pub pack_loader: String,
    /// The present mod bridging one of those loaders, when there is one. A bridge
    /// carries the mod -- what it does not promise is how stable the result is,
    /// and that rides on the connector rather than on any one mod, so it is not
    /// something this pass can judge per mod (and does not try to).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bridged_by: Option<String>,
}

/// A present mod that is depended-on but marked optional.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct RequiredHint {
    pub filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub modid: Option<String>,
    pub needed_by: Vec<String>,
}

/// A dependency on the loader itself (Forge/FML, NeoForge, Fabric, ...), which is
/// always present, so it is not a missing mod however the jar spells it -- Forge
/// mods variously require `forge`, `MinecraftForge`, `mod_MinecraftForge` or
/// `FML`. Any version window rides after `@` and is dropped first. A jar's loader
/// compatibility is judged separately (#50); this only stops the scanner
/// reporting the loader as a missing dependency (#10).
fn is_loader_dep(target: &str) -> bool {
    let id = target
        .split('@')
        .next()
        .unwrap_or(target)
        .to_ascii_lowercase();
    matches!(
        id.as_str(),
        "forge"
            | "minecraftforge"
            | "mod_minecraftforge"
            | "fml"
            | "forgemodloader"
            | "neoforge"
            | "fabric"
            | "fabricloader"
            | "cleanroom"
            | "quilt"
    )
}

/// A declared jar mod placed on the graph.
struct Present {
    filename: String,
    required: bool,
    /// Ships enabled unless the operator opted it out. With `required`, this says
    /// whether the mod is in the default install -- a conflict only bites when
    /// both sides actually run (#9).
    default_enabled: bool,
    mod_id: i64,
    version: Option<String>,
    /// The exact artifact the pack ships, when the registry has read it. A pack
    /// declares a file (by sha1, or by Modrinth version id), so its dependencies
    /// are that file's -- not the union of every version of its mod (#48). `None`
    /// when the artifact was never harvested: then only mod-level facts apply,
    /// since we have never actually looked inside this jar.
    mod_version_id: Option<i64>,
}

/// Place each declared jar mod on the registry graph, returning the ones that
/// landed plus the filenames that could not be identified. A `SmrtStatic` source
/// is not a mod (a config/asset file) and is skipped; a jar with no registry
/// identity cannot be reasoned about and is reported unresolved.
fn place_mods(conn: &Connection, cfg: &PackConfig) -> Result<PlacedMods> {
    let mut present: Vec<Present> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    let mut pinned_projects: Vec<String> = Vec::new();
    for m in &cfg.mods {
        let (mod_id, version, mod_version_id) = match &m.source {
            SourceDecl::SmrtCache { sha1 } => match queries::artifact_by_sha1(conn, sha1)? {
                Some((mv_id, id, ver)) => (id, Some(ver), Some(mv_id)),
                None => {
                    unresolved.push(m.filename.clone());
                    continue;
                }
            },
            SourceDecl::Modrinth {
                project_id,
                version_id,
            } => match queries::mod_id_for_alias(conn, "modrinth", project_id)? {
                Some(id) => (
                    id,
                    queries::version_by_modrinth_version_id(conn, version_id)?,
                    queries::mod_version_id_for_modrinth_version_id(conn, version_id)?,
                ),
                // A Modrinth pin the mirror has not harvested yet is still valid: a
                // build fetches it straight from Modrinth, so it will be present.
                // This is a pre-build check, so it must not be reported as an
                // unidentified mod. Record the project instead -- a
                // `modrinth:<project>` dependency resolves against it -- and leave
                // its own dependencies unchecked (nothing is harvested to read).
                None => {
                    pinned_projects.push(project_id.clone());
                    continue;
                }
            },
            SourceDecl::SmrtStatic { .. } => continue,
        };
        present.push(Present {
            filename: m.filename.clone(),
            required: m.required,
            default_enabled: m.default_enabled,
            mod_id,
            version,
            mod_version_id,
        });
    }
    Ok(PlacedMods {
        present,
        unresolved,
        pinned_projects,
    })
}

/// The outcome of placing a pack's declared mods on the registry graph.
struct PlacedMods {
    /// Mods the registry has an identity for -- reasoned about fully.
    present: Vec<Present>,
    /// Declared mods with no identity and no valid pin: an un-harvested `smrt_cache`
    /// upload. Listed, not judged.
    unresolved: Vec<String>,
    /// Modrinth project ids of declared Modrinth mods the mirror has not harvested.
    /// Valid pins (present at build); a `modrinth:<project>` dependency they cover
    /// is satisfied even though their own edges cannot be walked.
    pinned_projects: Vec<String>,
}

/// The relation graph of one pack: its own mods, wired by the relations the exact
/// artifacts it ships declare.
///
/// The same edges mean something different here than in the registry-wide graph.
/// Globally "X requires forge" is noise; inside a pack the question is whether the
/// thing required is actually here. So a target is resolved only when the pack
/// ships that mod -- anything else stays dangling, and the panel can read a
/// dangling `requires` as a missing dependency and a `conflicts` between two
/// present mods as a live one, off the same shape the registry graph already uses.
///
/// Unlike the registry graph this keeps every declared mod as a node, isolated or
/// not: a mod with no relations is still part of the pack, and this is a picture of
/// the pack's composition rather than of the relation web.
pub fn pack_graph(conn: &Connection, cfg: &PackConfig) -> Result<GraphData> {
    let present = place_mods(conn, cfg)?.present;
    let in_pack: HashSet<i64> = present.iter().map(|p| p.mod_id).collect();

    let mut nodes = Vec::with_capacity(present.len());
    let mut seen: HashSet<i64> = HashSet::new();
    for p in &present {
        if seen.insert(p.mod_id) {
            nodes.push(queries::graph_node_for(conn, p.mod_id)?);
        }
    }

    let mut edges = Vec::new();
    for p in &present {
        for e in queries::relations_for_artifact(conn, p.mod_version_id.unwrap_or(-1), p.mod_id)? {
            let to = queries::mod_id_for_selector(conn, &e.target)?.filter(|t| in_pack.contains(t));
            edges.push(GraphEdge {
                from_mod_id: p.mod_id,
                to_mod_id: to,
                target: e.target,
                kind: e.kind.as_str().to_string(),
                source: e.source.as_str().to_string(),
            });
        }
    }
    Ok(GraphData { nodes, edges })
}

/// Resolve `cfg` against the registry graph reachable through `conn`.
pub fn resolve_pack(conn: &Connection, cfg: &PackConfig) -> Result<ResolveReport> {
    // 1. Place each declared jar mod on the graph.
    let PlacedMods {
        present,
        mut unresolved,
        pinned_projects,
    } = place_mods(conn, cfg)?;
    // Modrinth projects the pack pins but the mirror has not harvested: a
    // `modrinth:<project>` dependency they cover is satisfied (present at build).
    let pinned: HashSet<&str> = pinned_projects.iter().map(String::as_str).collect();

    // first declaration of a mod_id wins the index (a pack rarely ships one mod
    // twice; if it does, the earlier row is the one findings point at)
    let mut by_mod_id: HashMap<i64, usize> = HashMap::new();
    for (i, p) in present.iter().enumerate() {
        by_mod_id.entry(p.mod_id).or_insert(i);
    }

    // 2. Walk each present mod's authoritative edges.
    let mut missing: BTreeMap<String, MissingDep> = BTreeMap::new();
    let mut conflicts: Vec<ActiveConflict> = Vec::new();
    let mut optional_conflicts: Vec<ActiveConflict> = Vec::new();
    let mut conflict_seen: HashSet<(usize, usize)> = HashSet::new();
    let mut provides: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut version_issues: Vec<VersionIssue> = Vec::new();
    let mut unchecked = 0usize;
    // depended-on present index -> the requirers, to hint required=false mistakes
    let mut depended_on: HashMap<usize, BTreeSet<String>> = HashMap::new();

    for (ai, a) in present.iter().enumerate() {
        // Scoped to the artifact the pack actually ships, plus the mod-level facts
        // (#48): a sibling version's dependencies are not this file's. `-1` matches
        // no artifact, which is how an unharvested jar falls back to mod-level
        // facts alone rather than borrowing another build's.
        //
        // Ordered by confidence, so the first edge per target is the authoritative
        // one -- an authored optional_dep suppresses an inferred requires for the
        // same target, and so on.
        let mut seen_target: HashSet<String> = HashSet::new();
        for e in queries::relations_for_artifact(conn, a.mod_version_id.unwrap_or(-1), a.mod_id)? {
            if !seen_target.insert(e.target.clone()) {
                continue;
            }
            match e.kind {
                RelKind::Requires => {
                    let tgt_present = queries::mod_id_for_selector(conn, &e.target)?
                        .and_then(|id| by_mod_id.get(&id).copied());
                    match tgt_present {
                        Some(bi) => {
                            depended_on
                                .entry(bi)
                                .or_default()
                                .insert(a.filename.clone());
                            if let Some(range) = e.version_range.as_deref() {
                                let b = &present[bi];
                                match b
                                    .version
                                    .as_deref()
                                    .and_then(|v| semver::in_range(v, range))
                                {
                                    Some(true) => {}
                                    Some(false) => version_issues.push(VersionIssue {
                                        target: e.target.clone(),
                                        filename: b.filename.clone(),
                                        present_version: b.version.clone().unwrap_or_default(),
                                        required_range: range.to_string(),
                                        needed_by: vec![a.filename.clone()],
                                    }),
                                    None => unchecked += 1,
                                }
                            }
                        }
                        None => {
                            // A dependency on the loader itself is never a missing
                            // mod -- the loader is always present (#10).
                            if is_loader_dep(&e.target) {
                                continue;
                            }
                            // A `modrinth:<project>` the pack pins but the mirror has
                            // not harvested is satisfied by that pin -- the build
                            // fetches it, so it is present, not missing.
                            if let Some(pid) = e.target.strip_prefix("modrinth:") {
                                let pid = pid.split('@').next().unwrap_or(pid);
                                if pinned.contains(pid) {
                                    continue;
                                }
                            }
                            let entry =
                                missing
                                    .entry(e.target.clone())
                                    .or_insert_with(|| MissingDep {
                                        target: e.target.clone(),
                                        needed_by: Vec::new(),
                                        version_range: e.version_range.clone(),
                                        source: e.source.as_str().to_string(),
                                    });
                            entry.needed_by.push(a.filename.clone());
                        }
                    }
                }
                RelKind::Conflicts | RelKind::Breaks => {
                    if let Some(bi) = queries::mod_id_for_selector(conn, &e.target)?
                        .and_then(|id| by_mod_id.get(&id).copied())
                    {
                        let pair = if ai < bi { (ai, bi) } else { (bi, ai) };
                        if conflict_seen.insert(pair) {
                            let c = ActiveConflict {
                                a: a.filename.clone(),
                                b: present[bi].filename.clone(),
                                breaks: matches!(e.kind, RelKind::Breaks),
                                source: e.source.as_str().to_string(),
                            };
                            // Both in the default install -> a live conflict; an
                            // opted-out optional on either side makes it advisory,
                            // firing only if the user enables that mod (#9).
                            let b = &present[bi];
                            let both_on = (a.required || a.default_enabled)
                                && (b.required || b.default_enabled);
                            if both_on {
                                conflicts.push(c);
                            } else {
                                optional_conflicts.push(c);
                            }
                        }
                    }
                }
                RelKind::Provides => {
                    provides
                        .entry(e.target.clone())
                        .or_default()
                        .insert(a.filename.clone());
                }
                // a soft dependency absent from the pack is the normal case, not a
                // problem to report
                RelKind::OptionalDep | RelKind::Recommends => {}
            }
        }
    }

    // A required target a present mod `provides` as a capability is satisfied.
    missing.retain(|target, _| !provides.contains_key(target));

    // Loader eligibility (#50). A pack natively runs its own loader and whatever
    // that loader inherits from; anything else needs a bridge. A bridge is a
    // present mod that `provides` the `loader:<name>` capability -- a connector.
    //
    // A connector carries the mod: bridged is not a warning, and does not spoil a
    // clean report. What a bridge does not promise is stability, and that depends
    // on the connector rather than on any one mod -- there is nothing to derive per
    // mod, so nothing is claimed. Listing them is still worth it: these are the
    // mods that all go at once if the connector leaves the pack.
    let chain = queries::loader_chain(conn, &cfg.loader.name)?;
    let mut loader_mismatch: Vec<LoaderMismatch> = Vec::new();
    let mut loader_bridged: Vec<LoaderMismatch> = Vec::new();
    for a in &present {
        // never read the jar -> its loaders are unknown, so there is nothing to judge
        let Some(mv) = a.mod_version_id else { continue };
        let targets = queries::targets_for_artifact(conn, mv)?;
        if targets.is_empty() {
            continue;
        }
        let native = targets
            .iter()
            .any(|t| t == "any" || chain.contains(&t.to_lowercase()));
        if native {
            continue;
        }
        let bridged_by = targets.iter().find_map(|t| {
            provides
                .get(&format!("loader:{}", t.to_lowercase()))
                .and_then(|by| by.iter().next().cloned())
        });
        let row = LoaderMismatch {
            filename: a.filename.clone(),
            artifact_loaders: targets,
            pack_loader: cfg.loader.name.clone(),
            bridged_by: bridged_by.clone(),
        };
        if bridged_by.is_some() {
            loader_bridged.push(row);
        } else {
            loader_mismatch.push(row);
        }
    }
    loader_mismatch.sort_by(|a, b| a.filename.cmp(&b.filename));
    loader_bridged.sort_by(|a, b| a.filename.cmp(&b.filename));

    let overlaps: Vec<CapabilityOverlap> = provides
        .into_iter()
        .filter(|(_, fns)| fns.len() >= 2)
        .map(|(capability, fns)| CapabilityOverlap {
            capability,
            mods: fns.into_iter().collect(),
        })
        .collect();

    let mut required_hints: Vec<RequiredHint> = depended_on
        .into_iter()
        .filter_map(|(bi, reqs)| {
            let p = &present[bi];
            if p.required {
                return None;
            }
            Some(RequiredHint {
                filename: p.filename.clone(),
                modid: queries::modid_for_mod(conn, p.mod_id).ok().flatten(),
                needed_by: reqs.into_iter().collect(),
            })
        })
        .collect();

    let mut missing: Vec<MissingDep> = missing
        .into_values()
        .map(|mut d| {
            d.needed_by.sort();
            d
        })
        .collect();
    missing.sort_by(|x, y| x.target.cmp(&y.target));
    conflicts.sort_by(|x, y| (&x.a, &x.b).cmp(&(&y.a, &y.b)));
    optional_conflicts.sort_by(|x, y| (&x.a, &x.b).cmp(&(&y.a, &y.b)));
    version_issues.sort_by(|x, y| (&x.filename, &x.target).cmp(&(&y.filename, &y.target)));
    required_hints.sort_by(|x, y| x.filename.cmp(&y.filename));
    unresolved.sort();
    unresolved.dedup();

    Ok(ResolveReport {
        declared_mods: cfg.mods.len(),
        // a Modrinth pin the mirror has not harvested is resolved too -- it is a
        // valid declaration the build will fetch, not an unidentified mod
        resolved_mods: present.len() + pinned_projects.len(),
        missing,
        conflicts,
        optional_conflicts,
        overlaps,
        version_issues,
        required_hints,
        loader_mismatch,
        loader_bridged,
        unresolved,
        version_windows_unchecked: unchecked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Display, LoaderSpec, PackConfig, PackTier, Visibility};
    use crate::registry::Registry;
    use crate::registry::upsert;

    const NOW: &str = "2026-07-15T00:00:00Z";

    fn declared(filename: &str, required: bool, source: SourceDecl) -> crate::domain::DeclaredMod {
        crate::domain::DeclaredMod {
            filename: filename.to_string(),
            required,
            default_enabled: true,
            source,
            display: None::<Display>,
            slug: None,
        }
    }

    fn cache(sha1: &str) -> SourceDecl {
        SourceDecl::SmrtCache {
            sha1: sha1.to_string(),
        }
    }

    fn config(mods: Vec<crate::domain::DeclaredMod>) -> PackConfig {
        PackConfig {
            pack_id: "test".into(),
            display_name: "Test".into(),
            tagline: String::new(),
            minecraft_version: "1.12.2".into(),
            loader: LoaderSpec {
                name: "forge".into(),
                version: "14.23.5.2860".into(),
            },
            java_major: 8,
            version: None,
            tags: vec![],
            featured: false,
            mods,
            assets: vec![],
            pack_meta: Default::default(),
            owner: 0,
            tier: PackTier::Official,
            visibility: Visibility::Published,
            fork_of: None,
        }
    }

    /// Register a mod (by modid) with one cached artifact; return nothing -- the
    /// pack refers to it by sha1.
    fn add_mod(r: &Registry, modid: &str, version: &str, sha1: &str) -> i64 {
        r.with_conn_mut(|c| {
            let id = upsert::upsert_mod_by_alias(c, &[("modid", modid)], NOW)?;
            upsert::upsert_mod_version(c, id, version, &["forge"], sha1, 10, None, None, NOW)?;
            Ok(id)
        })
        .unwrap()
    }

    fn relate(
        r: &Registry,
        from: i64,
        target: &str,
        range: Option<&str>,
        kind: RelKind,
        src: crate::registry::model::Source,
    ) {
        r.with_conn_mut(|c| {
            // mod-level: these fixtures assert resolver behaviour, not scoping
            upsert::upsert_relation(c, from, None, target, range, kind, src, NOW)?;
            Ok(())
        })
        .unwrap();
    }

    /// Register a mod whose artifact targets specific loaders.
    fn add_mod_for(r: &Registry, modid: &str, sha1: &str, loaders: &[&str]) -> i64 {
        r.with_conn_mut(|c| {
            let id = upsert::upsert_mod_by_alias(c, &[("modid", modid)], NOW)?;
            upsert::upsert_mod_version(c, id, "1.0", loaders, sha1, 10, None, None, NOW)?;
            Ok(id)
        })
        .unwrap()
    }

    // A pre-build check must recognise a valid Modrinth pin: litematica depends on
    // malilib by its Modrinth project, and the pack pins malilib from Modrinth but
    // the mirror has not harvested it. The build fetches it, so it is present -- the
    // dependency is satisfied and the pin is not an unidentified mod.
    #[test]
    fn a_modrinth_pin_satisfies_a_dependency_without_a_harvested_mod() {
        let r = Registry::open_in_memory().unwrap();
        let lite = add_mod(&r, "litematica", "1.0", "sha_lite");
        relate(
            &r,
            lite,
            "modrinth:malilib_proj",
            None,
            RelKind::Requires,
            crate::registry::model::Source::Modrinth,
        );
        let cfg = config(vec![
            declared("litematica.jar", true, cache("sha_lite")),
            declared(
                "malilib.jar",
                false,
                SourceDecl::Modrinth {
                    project_id: "malilib_proj".into(),
                    version_id: "v1".into(),
                },
            ),
        ]);
        let rep = r.with_conn(|c| resolve_pack(c, &cfg)).unwrap();
        assert!(
            rep.missing.is_empty(),
            "the pin satisfies the modrinth dependency: {:?}",
            rep.missing
        );
        assert!(
            rep.unresolved.is_empty(),
            "a valid Modrinth pin is not unresolved: {:?}",
            rep.unresolved
        );
        assert_eq!(
            rep.resolved_mods, 2,
            "the harvested mod and the pin both count as resolved"
        );
    }

    // #50: a fabric jar in a forge pack will not load, and nothing in the pack says
    // otherwise -- that is a fact worth flagging.
    #[test]
    fn foreign_loader_artifact_without_a_bridge_is_flagged() {
        let r = Registry::open_in_memory().unwrap();
        add_mod_for(&r, "fab", "sha_fab", &["fabric"]);
        let cfg = config(vec![declared("fab.jar", true, cache("sha_fab"))]); // config() is forge
        let rep = r.with_conn(|c| resolve_pack(c, &cfg)).unwrap();

        assert_eq!(rep.loader_mismatch.len(), 1);
        assert_eq!(rep.loader_mismatch[0].filename, "fab.jar");
        assert_eq!(rep.loader_mismatch[0].artifact_loaders, vec!["fabric"]);
        assert!(rep.loader_bridged.is_empty());
    }

    // A fork runs its parent's artifacts by construction, so a forge jar on a
    // cleanroom pack is not a mismatch at all (#37) -- and neither is an `any` jar.
    #[test]
    fn a_fork_runs_its_parents_artifacts_and_any_suits_everything() {
        let r = Registry::open_in_memory().unwrap();
        add_mod_for(&r, "fj", "sha_forge", &["forge"]);
        add_mod_for(&r, "tw", "sha_any", &["any"]);
        let mut cfg = config(vec![
            declared("forge.jar", true, cache("sha_forge")),
            declared("tweak.jar", true, cache("sha_any")),
        ]);
        cfg.loader.name = "cleanroom".into();
        let rep = r.with_conn(|c| resolve_pack(c, &cfg)).unwrap();
        assert!(
            rep.loader_mismatch.is_empty(),
            "cleanroom inherits forge, and `any` suits any loader"
        );
    }

    // A connector present in the pack carries the mod: not a finding, just a fact
    // worth listing -- pull the connector and every one of them goes at once.
    #[test]
    fn a_bridge_carries_a_foreign_artifact_and_is_not_a_finding() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        add_mod_for(&r, "fab", "sha_fab", &["fabric"]);
        let conn_mod = add_mod_for(&r, "connector", "sha_conn", &["forge"]);
        // the connector declares that it carries fabric's runtime
        relate(
            &r,
            conn_mod,
            "loader:fabric",
            None,
            RelKind::Provides,
            Source::Authored,
        );
        let cfg = config(vec![
            declared("fab.jar", true, cache("sha_fab")),
            declared("connector.jar", true, cache("sha_conn")),
        ]);
        let rep = r.with_conn(|c| resolve_pack(c, &cfg)).unwrap();

        assert!(
            rep.loader_mismatch.is_empty(),
            "a connector carries it: not a problem"
        );
        assert_eq!(rep.loader_bridged.len(), 1, "still worth listing");
        assert_eq!(rep.loader_bridged[0].filename, "fab.jar");
        assert_eq!(
            rep.loader_bridged[0].bridged_by.as_deref(),
            Some("connector.jar")
        );
    }

    // The pack graph is the same shape as the registry graph, but read against the
    // pack: a target the pack ships resolves, and one it does not stays dangling.
    // That is what lets the panel read a dangling `requires` as a missing dependency
    // and a `conflicts` between two present mods as a live one.
    #[test]
    fn pack_graph_resolves_only_what_the_pack_ships() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        let a = add_mod(&r, "aaa", "1.0", "sha_a");
        let b = add_mod(&r, "bbb", "1.0", "sha_b");
        add_mod(&r, "ccc", "1.0", "sha_c"); // in the registry, but not in this pack
        relate(&r, a, "bbb", None, RelKind::Requires, Source::JarMeta); // satisfied here
        relate(&r, a, "ccc", None, RelKind::Requires, Source::JarMeta); // not shipped
        relate(&r, a, "bbb", None, RelKind::Conflicts, Source::Authored); // live conflict
        let _ = b;

        let cfg = config(vec![
            declared("a.jar", true, cache("sha_a")),
            declared("b.jar", true, cache("sha_b")),
        ]);
        let g = r.with_conn(|c| pack_graph(c, &cfg)).unwrap();

        // every declared mod is a node, whether or not it has relations
        assert_eq!(g.nodes.len(), 2);

        let find = |target: &str, kind: &str| {
            g.edges
                .iter()
                .find(|e| e.target == target && e.kind == kind)
                .unwrap_or_else(|| panic!("no {kind} edge to {target}"))
        };
        assert!(
            find("bbb", "requires").to_mod_id.is_some(),
            "a requirement the pack ships resolves"
        );
        assert!(
            find("ccc", "requires").to_mod_id.is_none(),
            "a requirement the pack does not ship dangles -- that is the missing dep"
        );
        assert!(
            find("bbb", "conflicts").to_mod_id.is_some(),
            "a conflict between two shipped mods is live"
        );
    }

    // An artifact's own facts, not its mod's other versions (#48), decide what the
    // pack graph draws: shipping the old jar must not pull the new jar's dependency.
    #[test]
    fn pack_graph_follows_the_shipped_artifact() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        let (m, old, new) = r
            .with_conn_mut(|c| {
                let m = upsert::upsert_mod_by_alias(c, &[("modid", "mmm")], NOW)?;
                let old = upsert::upsert_mod_version(
                    c,
                    m,
                    "1.0",
                    &["forge"],
                    "sha_old",
                    1,
                    None,
                    None,
                    NOW,
                )?;
                let new = upsert::upsert_mod_version(
                    c,
                    m,
                    "2.0",
                    &["forge"],
                    "sha_new",
                    1,
                    None,
                    None,
                    NOW,
                )?;
                Ok((m, old, new))
            })
            .unwrap();
        add_mod(&r, "oldlib", "1.0", "sha_oldlib");
        add_mod(&r, "newlib", "1.0", "sha_newlib");
        r.with_conn_mut(|c| {
            upsert::upsert_relation(
                c,
                m,
                Some(old),
                "oldlib",
                None,
                RelKind::Requires,
                Source::JarMeta,
                NOW,
            )?;
            upsert::upsert_relation(
                c,
                m,
                Some(new),
                "newlib",
                None,
                RelKind::Requires,
                Source::JarMeta,
                NOW,
            )?;
            Ok(())
        })
        .unwrap();

        // the pack ships the OLD jar
        let cfg = config(vec![declared("m.jar", true, cache("sha_old"))]);
        let g = r.with_conn(|c| pack_graph(c, &cfg)).unwrap();
        let targets: Vec<&str> = g.edges.iter().map(|e| e.target.as_str()).collect();
        assert_eq!(
            targets,
            vec!["oldlib"],
            "the shipped artifact's dependency only -- the sibling version's is not this pack's"
        );
    }

    #[test]
    fn missing_hard_dep_is_flagged_when_target_absent() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        let stuff = add_mod(&r, "ae2stuff", "0.7.0", &"a".repeat(40));
        add_mod(&r, "appliedenergistics2", "0.44", &"b".repeat(40));
        relate(
            &r,
            stuff,
            "appliedenergistics2",
            None,
            RelKind::Requires,
            Source::Inferred,
        );

        // AE2 present -> satisfied
        let ok = r
            .with_conn(|c| {
                resolve_pack(
                    c,
                    &config(vec![
                        declared("ae2stuff.jar", true, cache(&"a".repeat(40))),
                        declared("ae2.jar", true, cache(&"b".repeat(40))),
                    ]),
                )
            })
            .unwrap();
        assert!(ok.missing.is_empty(), "AE2 present: {:?}", ok.missing);
        assert_eq!(ok.resolved_mods, 2);

        // AE2 removed -> missing
        let bad = r
            .with_conn(|c| {
                resolve_pack(
                    c,
                    &config(vec![declared("ae2stuff.jar", true, cache(&"a".repeat(40)))]),
                )
            })
            .unwrap();
        assert_eq!(bad.missing.len(), 1);
        assert_eq!(bad.missing[0].target, "appliedenergistics2");
        assert_eq!(bad.missing[0].needed_by, vec!["ae2stuff.jar"]);
    }

    #[test]
    fn authored_optional_suppresses_inferred_requires() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        let jei = add_mod(&r, "somemod", "1.0", &"c".repeat(40));
        // inferred says requires jei; authored says it's only optional
        relate(&r, jei, "jei", None, RelKind::Requires, Source::Inferred);
        relate(&r, jei, "jei", None, RelKind::OptionalDep, Source::Authored);

        let rep = r
            .with_conn(|c| {
                resolve_pack(
                    c,
                    &config(vec![declared("somemod.jar", true, cache(&"c".repeat(40)))]),
                )
            })
            .unwrap();
        assert!(
            rep.missing.is_empty(),
            "authored optional wins: {:?}",
            rep.missing
        );
    }

    #[test]
    fn active_conflict_between_two_present_mods() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        let a = add_mod(&r, "moda", "1.0", &"d".repeat(40));
        add_mod(&r, "modb", "1.0", &"e".repeat(40));
        relate(&r, a, "modb", None, RelKind::Conflicts, Source::Authored);

        let rep = r
            .with_conn(|c| {
                resolve_pack(
                    c,
                    &config(vec![
                        declared("a.jar", true, cache(&"d".repeat(40))),
                        declared("b.jar", true, cache(&"e".repeat(40))),
                    ]),
                )
            })
            .unwrap();
        assert_eq!(rep.conflicts.len(), 1);
        assert_eq!(rep.conflicts[0].a, "a.jar");
        assert_eq!(rep.conflicts[0].b, "b.jar");
        assert!(!rep.conflicts[0].breaks);
    }

    #[test]
    fn conflict_with_an_opted_out_optional_is_advisory() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        let a = add_mod(&r, "moda", "1.0", &"d".repeat(40));
        add_mod(&r, "modb", "1.0", &"e".repeat(40));
        relate(&r, a, "modb", None, RelKind::Conflicts, Source::Authored);

        // b.jar is an optional the pack ships disabled: the conflict only bites if
        // the user turns it on, so it is advisory, not a blocking conflict (#9).
        let b_off = crate::domain::DeclaredMod {
            filename: "b.jar".into(),
            required: false,
            default_enabled: false,
            source: cache(&"e".repeat(40)),
            display: None::<Display>,
            slug: None,
        };
        let rep = r
            .with_conn(|c| {
                resolve_pack(
                    c,
                    &config(vec![declared("a.jar", true, cache(&"d".repeat(40))), b_off]),
                )
            })
            .unwrap();
        assert!(
            rep.conflicts.is_empty(),
            "an opted-out optional is not a blocking conflict"
        );
        assert_eq!(rep.optional_conflicts.len(), 1);
        assert_eq!(rep.optional_conflicts[0].b, "b.jar");
    }

    #[test]
    fn a_loader_dependency_is_not_missing() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        let a = add_mod(&r, "moda", "1.0", &"d".repeat(40));
        // moda requires the loader, spelled the Forge way, with a version window
        relate(
            &r,
            a,
            "MinecraftForge",
            None,
            RelKind::Requires,
            Source::JarMeta,
        );

        let rep = r
            .with_conn(|c| {
                resolve_pack(
                    c,
                    &config(vec![declared("a.jar", true, cache(&"d".repeat(40)))]),
                )
            })
            .unwrap();
        assert!(
            rep.missing.is_empty(),
            "a dependency on the loader is not a missing mod (#10)"
        );
    }

    #[test]
    fn capability_overlap_and_provides_satisfaction() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        let ctm = add_mod(&r, "ctm", "1.0", &"1".repeat(40));
        let fusion = add_mod(&r, "fusion", "1.0", &"2".repeat(40));
        let user = add_mod(&r, "needsctm", "1.0", &"3".repeat(40));
        relate(
            &r,
            ctm,
            "connected_textures",
            None,
            RelKind::Provides,
            Source::Authored,
        );
        relate(
            &r,
            fusion,
            "connected_textures",
            None,
            RelKind::Provides,
            Source::Authored,
        );
        // a mod requiring the capability is satisfied by a provider, not "missing"
        relate(
            &r,
            user,
            "connected_textures",
            None,
            RelKind::Requires,
            Source::Authored,
        );

        let rep = r
            .with_conn(|c| {
                resolve_pack(
                    c,
                    &config(vec![
                        declared("ctm.jar", true, cache(&"1".repeat(40))),
                        declared("fusion.jar", true, cache(&"2".repeat(40))),
                        declared("user.jar", true, cache(&"3".repeat(40))),
                    ]),
                )
            })
            .unwrap();
        assert_eq!(rep.overlaps.len(), 1);
        assert_eq!(rep.overlaps[0].capability, "connected_textures");
        assert_eq!(rep.overlaps[0].mods, vec!["ctm.jar", "fusion.jar"]);
        assert!(
            rep.missing.is_empty(),
            "capability satisfies requires: {:?}",
            rep.missing
        );
    }

    #[test]
    fn required_hint_when_depended_on_but_optional() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        let stuff = add_mod(&r, "ae2stuff", "0.7", &"7".repeat(40));
        add_mod(&r, "appliedenergistics2", "0.44", &"8".repeat(40));
        relate(
            &r,
            stuff,
            "appliedenergistics2",
            None,
            RelKind::Requires,
            Source::Inferred,
        );

        // AE2 present but marked optional -> hint to make it required
        let rep = r
            .with_conn(|c| {
                resolve_pack(
                    c,
                    &config(vec![
                        declared("ae2stuff.jar", true, cache(&"7".repeat(40))),
                        declared("ae2.jar", false, cache(&"8".repeat(40))),
                    ]),
                )
            })
            .unwrap();
        assert_eq!(rep.required_hints.len(), 1);
        assert_eq!(rep.required_hints[0].filename, "ae2.jar");
        assert_eq!(
            rep.required_hints[0].modid.as_deref(),
            Some("appliedenergistics2")
        );
        assert_eq!(rep.required_hints[0].needed_by, vec!["ae2stuff.jar"]);
    }

    #[test]
    fn version_window_flagged_only_when_comparable() {
        use crate::registry::model::Source;
        let r = Registry::open_in_memory().unwrap();
        let dep = add_mod(&r, "usesnewlib", "1.0", &"9".repeat(40));
        add_mod(&r, "somelib", "1.0.0", &"0".repeat(40));
        relate(
            &r,
            dep,
            "somelib",
            Some("[2.0,)"),
            RelKind::Requires,
            Source::JarMeta,
        );

        let rep = r
            .with_conn(|c| {
                resolve_pack(
                    c,
                    &config(vec![
                        declared("uses.jar", true, cache(&"9".repeat(40))),
                        declared("lib.jar", true, cache(&"0".repeat(40))),
                    ]),
                )
            })
            .unwrap();
        assert_eq!(rep.version_issues.len(), 1);
        assert_eq!(rep.version_issues[0].filename, "lib.jar");
        assert_eq!(rep.version_issues[0].present_version, "1.0.0");
        assert_eq!(rep.version_issues[0].required_range, "[2.0,)");
    }

    #[test]
    fn unresolved_jar_is_listed_not_judged() {
        let r = Registry::open_in_memory().unwrap();
        // sha1 never harvested
        let rep = r
            .with_conn(|c| {
                resolve_pack(
                    c,
                    &config(vec![declared("ghost.jar", true, cache(&"f".repeat(40)))]),
                )
            })
            .unwrap();
        assert_eq!(rep.resolved_mods, 0);
        assert_eq!(rep.unresolved, vec!["ghost.jar"]);
        assert!(rep.missing.is_empty());
    }
}
