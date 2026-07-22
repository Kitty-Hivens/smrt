//! Dependency auto-fill, run when a pack config is saved. A curator adds the mods
//! they want; the mirror pulls in each mod's missing hard dependencies -- from
//! Modrinth first, else from its own cache -- so the operator never hand-manages
//! required libraries. It then records the resolved dependency graph in
//! `display.requires`, which is what the build reads to derive each mod's
//! required-ness (a dependency of a present mod is locked required; a top-level
//! mod stays optional unless its own classification requires it).

use super::modrinth::{Modrinth, Version as MrVersion};
use super::resolve;
use crate::domain::{DeclaredMod, Display, PackConfig, Requirement, SourceDecl};
use crate::registry::{Registry, queries, semver};
use anyhow::Result;
use std::collections::{HashMap, HashSet};

/// A dep chain deeper than this is almost certainly a resolution loop, not a real
/// tree; stop pulling rather than spin.
const MAX_PASSES: usize = 8;

/// Pull each declared mod's missing hard dependencies in (Modrinth first, the
/// mirror's own cache second), then write the resolved requires graph into
/// `display.requires`. A dep's own dependencies come in on the next pass; the
/// loop stops once a pass adds nothing. A dependency neither source can provide
/// is left for the resolve report to flag, not invented. `cached` is the live
/// cache inventory (sha1 set): only a jar whose bytes the mirror actually holds
/// can be declared as a `smrt_cache` source.
pub async fn fill_dependencies(
    cfg: &mut PackConfig,
    registry: &Registry,
    modrinth: &Modrinth,
    cached: &HashSet<String>,
) -> Result<usize> {
    let mut added_total = 0;
    for _ in 0..MAX_PASSES {
        let plan = {
            let cfg = &*cfg;
            registry.with_conn(|c| resolve::dependency_fill_plan(c, cfg))?
        };
        let mut added = false;
        for target in &plan.missing {
            // Per-target isolation: one unresolvable target (a Modrinth
            // outage, a dead project) must not abort the whole pass -- the
            // other targets still fill, and this one stays in the resolve
            // report's missing list instead of silently taking the rest
            // down with it.
            let decl = match resolve_target(target, cfg, registry, modrinth, cached).await {
                Ok(Some(d)) => d,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(
                        target = %target.selector,
                        error = %e,
                        "dependency target unresolved this pass; skipped"
                    );
                    continue;
                }
            };
            if !already_present(cfg, &decl) {
                cfg.mods.push(decl);
                added = true;
                added_total += 1;
            }
        }
        if !added {
            break;
        }
    }
    // record the final graph so the build derives required-ness from it
    let plan = {
        let cfg = &*cfg;
        registry.with_conn(|c| resolve::dependency_fill_plan(c, cfg))?
    };
    apply_requires(cfg, &plan.requires);
    prune_orphaned_pulled(cfg);
    Ok(added_total)
}

/// Sticky-dependency merge, run by the save path BEFORE the fill: every pulled
/// entry the previously-saved config carries and the incoming body lacks is
/// carried over. A client that never saw a server-pulled dependency (a stale
/// editor, a scripted PUT of a hand-written list) must not delete it -- and an
/// upstream outage during the following fill must not either, because the
/// entry no longer depends on being re-resolvable that moment. Curator-declared
/// entries are never resurrected: removing one is an explicit act.
pub fn merge_pulled(saved: &PackConfig, incoming: &mut PackConfig) {
    let mut present: HashSet<String> = HashSet::new();
    for m in &incoming.mods {
        present.insert(source_identity(&m.source));
        present.insert(format!("f:{}", m.filename));
    }
    for m in saved.mods.iter().filter(|m| m.pulled) {
        // matched by source identity OR filename: either means the incoming
        // body already carries this dependency in some form
        if !present.contains(&source_identity(&m.source))
            && !present.contains(&format!("f:{}", m.filename))
        {
            incoming.mods.push(m.clone());
        }
    }
}

/// The identity a pulled entry is matched by across saves: the Modrinth
/// project (a re-pin to another version is still the same dependency), the
/// cache sha1, the static path.
fn source_identity(s: &SourceDecl) -> String {
    match s {
        SourceDecl::Modrinth { project_id, .. } => format!("m:{project_id}"),
        SourceDecl::SmrtCache { sha1 } => format!("c:{sha1}"),
        SourceDecl::SmrtStatic { rel_path } => format!("s:{rel_path}"),
    }
}

/// Drop pulled entries no curator-declared mod still reaches through hard
/// requires edges. Reachability, not per-edge presence, so a chain of pulled
/// libraries (A -> lib1 -> lib2) lives exactly as long as its curator root
/// does. Runs after `apply_requires`, which derives the edges from the
/// registry locally -- available in any upstream weather.
fn prune_orphaned_pulled(cfg: &mut PackConfig) {
    let hard_edges: HashMap<&str, Vec<&str>> = cfg
        .mods
        .iter()
        .map(|m| {
            let targets = m
                .display
                .as_ref()
                .map(|d| {
                    d.requires
                        .iter()
                        .filter(|r| !r.optional)
                        .map(|r| r.filename.as_str())
                        .collect()
                })
                .unwrap_or_default();
            (m.filename.as_str(), targets)
        })
        .collect();
    let mut reachable: HashSet<String> = HashSet::new();
    let mut queue: Vec<&str> = cfg
        .mods
        .iter()
        .filter(|m| !m.pulled)
        .map(|m| m.filename.as_str())
        .collect();
    while let Some(f) = queue.pop() {
        if reachable.insert(f.to_string())
            && let Some(targets) = hard_edges.get(f)
        {
            queue.extend(targets.iter().copied());
        }
    }
    let before = cfg.mods.len();
    cfg.mods
        .retain(|m| !m.pulled || reachable.contains(&m.filename));
    let dropped = before - cfg.mods.len();
    if dropped > 0 {
        tracing::info!(
            dropped,
            "pruned pulled dependencies nothing declares anymore"
        );
    }
}

/// Resolve a missing dependency to a declarable source, by priority: a Modrinth
/// version for the pack's Minecraft version and loader, else a cached artifact
/// of the mod the selector names (checked against the requirer's version window
/// where one is declared and comparable). `None` when neither source can
/// provide it -- the resolve report flags it instead.
async fn resolve_target(
    target: &resolve::MissingTarget,
    cfg: &PackConfig,
    registry: &Registry,
    modrinth: &Modrinth,
    cached: &HashSet<String>,
) -> Result<Option<DeclaredMod>> {
    let bare = target
        .selector
        .split('@')
        .next()
        .unwrap_or(&target.selector);
    if bare.starts_with("external:") {
        return Ok(None); // outside both ecosystems by definition
    }
    let project = match bare.strip_prefix("modrinth:") {
        Some(p) => Some(p.to_string()),
        None => {
            let sel = bare.to_string();
            registry.with_conn(move |c| {
                Ok(queries::mod_id_for_selector(c, &sel)?
                    .and_then(|id| queries::modrinth_id_for_mod(c, id).ok().flatten()))
            })?
        }
    };
    if let Some(project) = project {
        let loader = cfg.loader.name.to_ascii_lowercase();
        let listing = modrinth
            .project_versions(&project, Some(&cfg.minecraft_version))
            .await?;
        // Modrinth returns versions newest-first, so the first usable one is the
        // latest compatible build.
        if let Some(v) = listing.into_iter().find(|v| usable(v, &loader)) {
            return Ok(Some(pulled_from_version(&project, &v)));
        }
    }
    // Modrinth cannot provide it: fall back to the mirror's own cache.
    resolve_from_cache(target, cfg, registry, cached)
}

/// Whether a Modrinth version can actually be declared: it runs on the pack's
/// loader, and upstream published a file for it. A version with an empty `files`
/// array is a broken publish (the metadata landed, the jar did not) -- pinning it
/// would only fail at build time, so it is never chosen.
fn usable(v: &MrVersion, loader: &str) -> bool {
    v.loaders.iter().any(|l| l.eq_ignore_ascii_case(loader)) && v.primary_file().is_some()
}

/// A usable Modrinth version as a pulled declaration.
fn pulled_from_version(project: &str, v: &MrVersion) -> DeclaredMod {
    DeclaredMod {
        filename: v
            .primary_file()
            .map(|f| f.filename.clone())
            .unwrap_or_else(|| format!("{project}.jar")),
        default_enabled: true,
        source: SourceDecl::Modrinth {
            project_id: project.to_string(),
            version_id: v.id.clone(),
        },
        display: None,
        slug: None,
        pulled: true,
    }
}

/// The cache leg of the chain: the selector's mod, its cached artifacts,
/// narrowed to the pack's loader family and Minecraft version, the requirer's
/// version window applied where comparable, newest surviving artifact wins.
fn resolve_from_cache(
    target: &resolve::MissingTarget,
    cfg: &PackConfig,
    registry: &Registry,
    cached: &HashSet<String>,
) -> Result<Option<DeclaredMod>> {
    let selector = target.selector.clone();
    let range = target.version_range.clone();
    let loader = cfg.loader.name.to_ascii_lowercase();
    let mc = cfg.minecraft_version.clone();
    let cached = cached.clone();
    registry.with_conn(move |c| {
        let Some(mod_id) = queries::mod_id_for_selector(c, &selector)? else {
            return Ok(None);
        };
        let chain = queries::loader_chain(c, &loader)?;
        let mut best: Option<(i64, DeclaredMod)> = None;
        for (i, v) in queries::versions_of_mod_by_id(c, mod_id)?
            .into_iter()
            .enumerate()
        {
            if !cached.contains(&v.sha1) {
                continue;
            }
            let loader_ok = v
                .targets
                .iter()
                .any(|t| t == "any" || chain.contains(&t.to_lowercase()));
            let mc_ok = v.mc_versions.is_empty() || v.mc_versions.contains(&mc);
            if !loader_ok || !mc_ok {
                continue;
            }
            // the requirer's window: reject a plainly out-of-window artifact;
            // an incomparable version passes (never act on a guess)
            if let Some(r) = range.as_deref()
                && semver::in_range(&v.version, r) == Some(false)
            {
                continue;
            }
            let filename = v
                .filename
                .clone()
                .unwrap_or_else(|| format!("{}.jar", v.sha1));
            let decl = DeclaredMod {
                filename,
                default_enabled: true,
                source: SourceDecl::SmrtCache {
                    sha1: v.sha1.clone(),
                },
                display: None,
                slug: None,
                pulled: true,
            };
            // rows come version-ordered; keep the last acceptable one (newest)
            best = Some((i as i64, decl));
        }
        Ok(best.map(|(_, d)| d))
    })
}

/// A pulled dependency is already in the pack when its source identity is
/// declared -- the Modrinth project id or the cache sha1, not the filename, so
/// a dep is not re-added under a different display name.
fn already_present(cfg: &PackConfig, decl: &DeclaredMod) -> bool {
    match &decl.source {
        SourceDecl::Modrinth { project_id, .. } => cfg.mods.iter().any(
            |m| matches!(&m.source, SourceDecl::Modrinth { project_id: p, .. } if p == project_id),
        ),
        SourceDecl::SmrtCache { sha1 } => cfg
            .mods
            .iter()
            .any(|m| matches!(&m.source, SourceDecl::SmrtCache { sha1: s } if s == sha1)),
        SourceDecl::SmrtStatic { .. } => false,
    }
}

/// Overwrite each mod's `display.requires` with the resolved present-mod edges, so
/// the field mirrors the registry graph exactly (no stale hand-entered deps linger).
fn apply_requires(cfg: &mut PackConfig, requires: &[(String, String)]) {
    let mut by_from: HashMap<&str, Vec<String>> = HashMap::new();
    for (from, dep) in requires {
        by_from.entry(from.as_str()).or_default().push(dep.clone());
    }
    for m in &mut cfg.mods {
        let mut deps = by_from
            .get(m.filename.as_str())
            .cloned()
            .unwrap_or_default();
        deps.sort();
        deps.dedup();
        if deps.is_empty() {
            // leave a hand-authored display untouched apart from clearing its stale
            // requires; do not materialize a display just to hold an empty list
            if let Some(d) = &mut m.display {
                d.requires.clear();
            }
            continue;
        }
        let reqs = deps
            .into_iter()
            .map(|filename| Requirement {
                filename,
                version_range: None,
                optional: false,
            })
            .collect();
        m.display.get_or_insert_with(Display::default).requires = reqs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LoaderSpec;
    use crate::domain::pack::{default_owner, default_tier, default_visibility};

    fn one(filename: &str) -> DeclaredMod {
        DeclaredMod {
            filename: filename.into(),
            default_enabled: true,
            source: SourceDecl::SmrtCache {
                sha1: "0".repeat(40),
            },
            display: None,
            slug: None,
            pulled: false,
        }
    }

    fn cfg(mods: Vec<DeclaredMod>) -> PackConfig {
        PackConfig {
            pack_id: "t".into(),
            display_name: "t".into(),
            tagline: String::new(),
            minecraft_version: "1.21.1".into(),
            loader: LoaderSpec {
                name: "neoforge".into(),
                version: String::new(),
            },
            java_major: 21,
            version: None,
            tags: vec![],
            featured: false,
            mods,
            assets: vec![],
            auth: None,
            pack_meta: Default::default(),
            owner: default_owner(),
            tier: default_tier(),
            visibility: default_visibility(),
            fork_of: None,
        }
    }

    use crate::registry::model::{RelKind, Source};
    use crate::registry::{Registry, upsert};

    const NOW: &str = "2026-07-18T00:00:00Z";

    fn cache_mod(filename: &str, sha: &str) -> DeclaredMod {
        DeclaredMod {
            filename: filename.into(),
            default_enabled: true,
            source: SourceDecl::SmrtCache { sha1: sha.into() },
            display: None,
            slug: None,
            pulled: false,
        }
    }

    fn add_artifact(r: &Registry, modid: &str, version: &str, sha: &str, filename: &str) -> i64 {
        r.with_conn_mut(|c| {
            let id = upsert::upsert_mod_by_alias(c, &[("modid", modid)], NOW)?;
            upsert::upsert_mod_version(
                c,
                id,
                version,
                &["forge"],
                sha,
                10,
                Some(filename),
                None,
                NOW,
            )?;
            Ok(id)
        })
        .unwrap()
    }

    // The cache leg of the source chain: a hard dependency Modrinth cannot
    // provide is pulled from the mirror's own cache, once, and a re-run adds
    // nothing (deduped by sha1).
    #[tokio::test]
    async fn cache_fallback_pulls_a_cached_artifact() {
        let r = Registry::open_in_memory().unwrap();
        let a = add_artifact(&r, "moda", "1.0", "sha_a", "a.jar");
        add_artifact(&r, "modb", "1.0", "sha_b", "modb-1.0.jar");
        r.with_conn_mut(|c| {
            upsert::upsert_relation(
                c,
                a,
                None,
                "modb",
                None,
                RelKind::Requires,
                Source::JarMeta,
                NOW,
            )?;
            Ok(())
        })
        .unwrap();
        let mut c = cfg(vec![cache_mod("a.jar", "sha_a")]);
        c.loader.name = "forge".into();
        let modrinth = Modrinth::new().unwrap();
        let cached: HashSet<String> = ["sha_a", "sha_b"].iter().map(|s| s.to_string()).collect();

        let added = fill_dependencies(&mut c, &r, &modrinth, &cached)
            .await
            .unwrap();
        assert_eq!(added, 1, "the cached dependency is pulled");
        let pulled = c
            .mods
            .iter()
            .find(|m| m.filename == "modb-1.0.jar")
            .unwrap();
        assert!(
            matches!(&pulled.source, SourceDecl::SmrtCache { sha1 } if sha1 == "sha_b"),
            "declared as a smrt_cache source"
        );
        // the requires edge landed so the build locks the pulled dep
        let reqs = &c.mods[0].display.as_ref().unwrap().requires;
        assert_eq!(reqs[0].filename, "modb-1.0.jar");

        let again = fill_dependencies(&mut c, &r, &modrinth, &cached)
            .await
            .unwrap();
        assert_eq!(again, 0, "idempotent: nothing re-added");
    }

    // The sticky-dependency contract: a pulled entry survives a save body that
    // lacks it, survives an upstream outage during the fill, and dies exactly
    // when nothing declared reaches it anymore.
    #[tokio::test]
    async fn pulled_dependencies_stick_through_outages_and_prune_as_orphans() {
        let r = Registry::open_in_memory().unwrap();
        let a = add_artifact(&r, "moda", "1.0", "sha_a", "a.jar");
        add_artifact(&r, "modb", "1.0", "sha_b", "modb-1.0.jar");
        r.with_conn_mut(|c| {
            upsert::upsert_relation(
                c,
                a,
                None,
                "modb",
                None,
                RelKind::Requires,
                Source::JarMeta,
                NOW,
            )?;
            Ok(())
        })
        .unwrap();

        // the previously-saved config: curator mod + its pulled dependency
        let mut saved = cfg(vec![cache_mod("a.jar", "sha_a")]);
        saved.loader.name = "forge".into();
        let mut dep = cache_mod("modb-1.0.jar", "sha_b");
        dep.source = SourceDecl::SmrtCache {
            sha1: "sha_b".into(),
        };
        dep.pulled = true;
        saved.mods.push(dep);

        // a stale client body: the curator mod only -- and Modrinth is down
        let mut incoming = cfg(vec![cache_mod("a.jar", "sha_a")]);
        incoming.loader.name = "forge".into();
        merge_pulled(&saved, &mut incoming);
        assert!(
            incoming.mods.iter().any(|m| m.filename == "modb-1.0.jar"),
            "the pulled dependency is carried over from the saved config"
        );

        let modrinth = Modrinth::with_base("http://127.0.0.1:9").unwrap();
        let cached: HashSet<String> = ["sha_a", "sha_b"].iter().map(|s| s.to_string()).collect();
        fill_dependencies(&mut incoming, &r, &modrinth, &cached)
            .await
            .unwrap();
        let kept = incoming
            .mods
            .iter()
            .find(|m| m.filename == "modb-1.0.jar")
            .expect("outage weather must not drop a previously-resolved dependency");
        assert!(kept.pulled, "the sticky marker survives the round trip");

        // a curator-declared mod removed from the body is NOT resurrected
        let mut without_curator = cfg(vec![]);
        without_curator.loader.name = "forge".into();
        merge_pulled(&saved, &mut without_curator);
        assert!(
            !without_curator.mods.iter().any(|m| m.filename == "a.jar"),
            "removing a curator mod is an explicit act"
        );
        // ...and once the dependent is gone, the orphaned pulled dep prunes
        fill_dependencies(&mut without_curator, &r, &modrinth, &cached)
            .await
            .unwrap();
        assert!(
            without_curator.mods.is_empty(),
            "a pulled dependency nothing reaches is dropped"
        );
    }

    // merge matches by source identity OR filename, so a re-pin of the same
    // project (new version, new filename) never duplicates the dependency.
    #[test]
    fn merge_does_not_duplicate_a_repinned_project() {
        let mk = |version_id: &str, filename: &str, pulled: bool| DeclaredMod {
            filename: filename.into(),
            default_enabled: true,
            source: SourceDecl::Modrinth {
                project_id: "PROJ".into(),
                version_id: version_id.into(),
            },
            display: None,
            slug: None,
            pulled,
        };
        let saved = cfg(vec![mk("v1", "lib-1.0.jar", true)]);
        let mut incoming = cfg(vec![mk("v2", "lib-2.0.jar", false)]);
        merge_pulled(&saved, &mut incoming);
        assert_eq!(
            incoming.mods.len(),
            1,
            "same project under a new pin is the same dependency"
        );
    }

    // One unresolvable target (here: a Modrinth-aliased dep with the API
    // unreachable) must not abort the pass -- the cache-resolvable dependency
    // still fills, and the fill itself reports success.
    #[tokio::test]
    async fn an_unreachable_target_does_not_abort_the_pass() {
        let r = Registry::open_in_memory().unwrap();
        let a = add_artifact(&r, "moda", "1.0", "sha_a", "a.jar");
        add_artifact(&r, "modb", "1.0", "sha_b", "modb-1.0.jar");
        r.with_conn_mut(|c| {
            // netlib resolves through Modrinth (it carries a project alias)
            let netlib = upsert::upsert_mod_by_alias(
                c,
                &[("modid", "netlib"), ("modrinth", "AAAAAAAA")],
                NOW,
            )?;
            let _ = netlib;
            upsert::upsert_relation(
                c,
                a,
                None,
                "netlib",
                None,
                RelKind::Requires,
                Source::JarMeta,
                NOW,
            )?;
            upsert::upsert_relation(
                c,
                a,
                None,
                "modb",
                None,
                RelKind::Requires,
                Source::JarMeta,
                NOW,
            )?;
            Ok(())
        })
        .unwrap();
        let mut c = cfg(vec![cache_mod("a.jar", "sha_a")]);
        c.loader.name = "forge".into();
        // nothing listens here: the Modrinth leg fails fast with a connect error
        let modrinth = Modrinth::with_base("http://127.0.0.1:9").unwrap();
        let cached: HashSet<String> = ["sha_a", "sha_b"].iter().map(|s| s.to_string()).collect();

        let added = fill_dependencies(&mut c, &r, &modrinth, &cached)
            .await
            .expect("a dead target must not fail the whole fill");
        assert_eq!(added, 1, "the cache-resolvable dependency still fills");
        assert!(
            c.mods.iter().any(|m| m.filename == "modb-1.0.jar"),
            "modb pulled despite the netlib failure"
        );
    }

    // A cached artifact plainly outside the requirer's version window is not
    // pulled; a jar whose bytes are not actually in the cache never is.
    #[tokio::test]
    async fn cache_fallback_respects_window_and_inventory() {
        let r = Registry::open_in_memory().unwrap();
        let a = add_artifact(&r, "moda", "1.0", "sha_a", "a.jar");
        add_artifact(&r, "oldlib", "1.0", "sha_old", "oldlib-1.0.jar");
        r.with_conn_mut(|c| {
            upsert::upsert_relation(
                c,
                a,
                None,
                "oldlib",
                Some("[2.0,)"),
                RelKind::Requires,
                Source::JarMeta,
                NOW,
            )?;
            Ok(())
        })
        .unwrap();
        let mut c = cfg(vec![cache_mod("a.jar", "sha_a")]);
        c.loader.name = "forge".into();
        let modrinth = Modrinth::new().unwrap();

        let cached: HashSet<String> = ["sha_a", "sha_old"].iter().map(|s| s.to_string()).collect();
        let added = fill_dependencies(&mut c, &r, &modrinth, &cached)
            .await
            .unwrap();
        assert_eq!(added, 0, "1.0 is outside [2.0,): not pulled");

        // and without the bytes in the cache inventory, nothing to declare
        let sparse: HashSet<String> = ["sha_a".to_string()].into_iter().collect();
        let added = fill_dependencies(&mut c, &r, &modrinth, &sparse)
            .await
            .unwrap();
        assert_eq!(added, 0);
    }

    // The client-mod guard holds on the pull path: an inferred hard edge into a
    // client-side mod pulls nothing, so a client mod can never arrive locked.
    #[tokio::test]
    async fn client_side_dep_is_not_pulled() {
        let r = Registry::open_in_memory().unwrap();
        let a = add_artifact(&r, "chisel", "1.0", "sha_a", "chisel.jar");
        add_artifact(&r, "ctm", "1.0", "sha_ctm", "ctm.jar");
        r.with_conn_mut(|c| {
            upsert::set_jar_class(c, "sha_ctm", "mod", Some("client"), Some("tolerant"), None)?;
            upsert::upsert_relation(
                c,
                a,
                None,
                "ctm",
                None,
                RelKind::Requires,
                Source::Inferred,
                NOW,
            )?;
            Ok(())
        })
        .unwrap();
        let mut c = cfg(vec![cache_mod("chisel.jar", "sha_a")]);
        c.loader.name = "forge".into();
        let modrinth = Modrinth::new().unwrap();
        let cached: HashSet<String> = ["sha_a", "sha_ctm"].iter().map(|s| s.to_string()).collect();
        let added = fill_dependencies(&mut c, &r, &modrinth, &cached)
            .await
            .unwrap();
        assert_eq!(added, 0, "a client-side mod is never auto-pulled");
        assert_eq!(c.mods.len(), 1);
    }

    // Recommends never lands in the config by itself: it rides the fill plan's
    // suggested list for the panel to offer.
    #[tokio::test]
    async fn recommends_is_suggested_not_added() {
        let r = Registry::open_in_memory().unwrap();
        let a = add_artifact(&r, "moda", "1.0", "sha_a", "a.jar");
        add_artifact(&r, "modr", "1.0", "sha_r", "modr.jar");
        r.with_conn_mut(|c| {
            upsert::upsert_relation(
                c,
                a,
                None,
                "modr",
                None,
                RelKind::Recommends,
                Source::JarMeta,
                NOW,
            )?;
            Ok(())
        })
        .unwrap();
        let mut c = cfg(vec![cache_mod("a.jar", "sha_a")]);
        c.loader.name = "forge".into();
        let modrinth = Modrinth::new().unwrap();
        let cached: HashSet<String> = ["sha_a", "sha_r"].iter().map(|s| s.to_string()).collect();
        let added = fill_dependencies(&mut c, &r, &modrinth, &cached)
            .await
            .unwrap();
        assert_eq!(added, 0, "recommends is never auto-added");
        let plan = r
            .with_conn(|conn| resolve::dependency_fill_plan(conn, &c))
            .unwrap();
        assert_eq!(plan.suggested, vec!["modr".to_string()]);
    }

    fn version_json(project: &str, id: &str, filename: &str, deps: &str) -> String {
        format!(
            r#"{{"id":"{id}","project_id":"{project}","name":"n","version_number":"1.0",
               "version_type":"release","game_versions":["1.21.1"],"loaders":["neoforge"],
               "files":[{{"hashes":{{"sha1":"{}"}},"url":"http://x/{filename}",
                 "filename":"{filename}","primary":true,"size":10}}],
               "dependencies":[{deps}]}}"#,
            "a".repeat(40)
        )
    }

    // Upstream sometimes publishes a version whose jar never landed (metadata
    // listed, `files` empty). Pinning one only fails at build time, so it is not
    // a candidate however new it is.
    #[test]
    fn a_version_without_a_file_is_never_usable() {
        let with_file: MrVersion =
            serde_json::from_str(&version_json("P", "V", "m.jar", "")).unwrap();
        assert!(usable(&with_file, "neoforge"));
        assert!(!usable(&with_file, "fabric"), "wrong loader");

        let mut fileless = with_file.clone();
        fileless.files.clear();
        assert!(
            !usable(&fileless, "neoforge"),
            "a version upstream published without a jar is not a candidate"
        );
    }

    #[test]
    fn apply_requires_records_edges_and_leaves_leaves_bare() {
        let mut c = cfg(vec![one("a.jar"), one("lib.jar")]);
        apply_requires(&mut c, &[("a.jar".into(), "lib.jar".into())]);
        let a = c.mods.iter().find(|m| m.filename == "a.jar").unwrap();
        let reqs = &a.display.as_ref().unwrap().requires;
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].filename, "lib.jar");
        assert!(!reqs[0].optional);
        // a mod that depends on nothing does not get a display materialized just to
        // hold an empty requires list
        let lib = c.mods.iter().find(|m| m.filename == "lib.jar").unwrap();
        assert!(lib.display.is_none());
    }
}
