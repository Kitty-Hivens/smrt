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
//! The graph is mod-level, not artifact-level: an edge belongs to a mod, derived
//! across whatever jars were harvested for it, so the resolver reasons at mod
//! granularity and does not re-scope edges to the pack's exact loader/mc.

use crate::domain::{PackConfig, SourceDecl};
use crate::registry::model::RelKind;
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
    /// How many declared jar mods mapped to a registry identity (the rest are in
    /// `unresolved` and could not be reasoned about).
    #[ts(type = "number")]
    pub resolved_mods: usize,
    /// A hard dependency no present mod satisfies -- the pack would crash.
    pub missing: Vec<MissingDep>,
    /// Two present mods the graph says cannot run together.
    pub conflicts: Vec<ActiveConflict>,
    /// A capability more than one present mod provides -- usually redundant, and
    /// the two may fight over the same hook.
    pub overlaps: Vec<CapabilityOverlap>,
    /// A present dependency whose shipped version sits outside a requirer's
    /// declared window.
    pub version_issues: Vec<VersionIssue>,
    /// A present mod that another present mod requires but that the pack marks
    /// optional -- it should be required.
    pub required_hints: Vec<RequiredHint>,
    /// Declared jar mods with no registry identity yet (an un-harvested upload,
    /// or a Modrinth pin the mirror has not seen). Left unjudged, listed so the
    /// operator knows coverage was partial.
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

/// A declared jar mod placed on the graph.
struct Present {
    filename: String,
    required: bool,
    mod_id: i64,
    version: Option<String>,
    /// The exact artifact the pack ships, when the registry has read it. A pack
    /// declares a file (by sha1, or by Modrinth version id), so its dependencies
    /// are that file's -- not the union of every version of its mod (#48). `None`
    /// when the artifact was never harvested: then only mod-level facts apply,
    /// since we have never actually looked inside this jar.
    mod_version_id: Option<i64>,
}

/// Resolve `cfg` against the registry graph reachable through `conn`.
pub fn resolve_pack(conn: &Connection, cfg: &PackConfig) -> Result<ResolveReport> {
    // 1. Place each declared jar mod on the graph. A SmrtStatic source is not a
    //    mod (config/asset file); a jar with no registry identity is unresolved.
    let mut present: Vec<Present> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
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
                None => {
                    unresolved.push(m.filename.clone());
                    continue;
                }
            },
            SourceDecl::SmrtStatic { .. } => continue,
        };
        present.push(Present {
            filename: m.filename.clone(),
            required: m.required,
            mod_id,
            version,
            mod_version_id,
        });
    }

    // first declaration of a mod_id wins the index (a pack rarely ships one mod
    // twice; if it does, the earlier row is the one findings point at)
    let mut by_mod_id: HashMap<i64, usize> = HashMap::new();
    for (i, p) in present.iter().enumerate() {
        by_mod_id.entry(p.mod_id).or_insert(i);
    }

    // 2. Walk each present mod's authoritative edges.
    let mut missing: BTreeMap<String, MissingDep> = BTreeMap::new();
    let mut conflicts: Vec<ActiveConflict> = Vec::new();
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
                            conflicts.push(ActiveConflict {
                                a: a.filename.clone(),
                                b: present[bi].filename.clone(),
                                breaks: matches!(e.kind, RelKind::Breaks),
                                source: e.source.as_str().to_string(),
                            });
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
    version_issues.sort_by(|x, y| (&x.filename, &x.target).cmp(&(&y.filename, &y.target)));
    required_hints.sort_by(|x, y| x.filename.cmp(&y.filename));
    unresolved.sort();
    unresolved.dedup();

    Ok(ResolveReport {
        declared_mods: cfg.mods.len(),
        resolved_mods: present.len(),
        missing,
        conflicts,
        overlaps,
        version_issues,
        required_hints,
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
            note: None,
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
