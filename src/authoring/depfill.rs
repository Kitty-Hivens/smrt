//! Dependency auto-fill, run when a pack config is saved. A curator adds the mods
//! they want; the mirror pulls in each mod's missing hard dependencies from
//! Modrinth so the operator never hand-manages required libraries. It then records
//! the resolved dependency graph in `display.requires`, which is what the build
//! reads to derive each mod's required-ness (a dependency of a present mod is
//! locked required; a top-level mod stays optional).

use super::modrinth::Modrinth;
use super::resolve;
use crate::domain::{DeclaredMod, Display, PackConfig, Requirement, SourceDecl};
use crate::registry::{Registry, queries};
use anyhow::Result;
use std::collections::HashMap;

/// A dep chain deeper than this is almost certainly a resolution loop, not a real
/// tree; stop pulling rather than spin.
const MAX_PASSES: usize = 8;

/// Pull each declared mod's missing hard dependencies in from Modrinth, then write
/// the resolved requires graph into `display.requires`. A dep's own dependencies
/// come in on the next pass; the loop stops once a pass adds nothing. A dependency
/// the registry cannot resolve to a source is left for the resolve report to flag,
/// not invented.
pub async fn fill_dependencies(
    cfg: &mut PackConfig,
    registry: &Registry,
    modrinth: &Modrinth,
) -> Result<usize> {
    let mut added_total = 0;
    for _ in 0..MAX_PASSES {
        let plan = {
            let cfg = &*cfg;
            registry.with_conn(|c| resolve::dependency_fill_plan(c, cfg))?
        };
        let mut added = false;
        for target in &plan.missing {
            let Some(decl) = resolve_target(target, cfg, registry, modrinth).await? else {
                continue;
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
    Ok(added_total)
}

/// Resolve a missing dependency selector to a Modrinth mod for the pack's Minecraft
/// version and loader. A `modrinth:<project>` names the project directly; a bare
/// modid resolves through the registry's Modrinth alias for that mod. `None` when
/// the mirror has no Modrinth project for it (a self-hosted-only dep the operator
/// must add by hand) or Modrinth carries no matching version.
async fn resolve_target(
    target: &str,
    cfg: &PackConfig,
    registry: &Registry,
    modrinth: &Modrinth,
) -> Result<Option<DeclaredMod>> {
    let bare = target.split('@').next().unwrap_or(target);
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
    let Some(project) = project else {
        return Ok(None);
    };
    let loader = cfg.loader.name.to_ascii_lowercase();
    let versions = modrinth
        .project_versions(&project, Some(&cfg.minecraft_version))
        .await?;
    // Modrinth returns versions newest-first, so the first that suits the loader is
    // the latest compatible build.
    let Some(v) = versions
        .into_iter()
        .find(|v| v.loaders.iter().any(|l| l.eq_ignore_ascii_case(&loader)))
    else {
        return Ok(None);
    };
    let filename = v
        .primary_file()
        .map(|f| f.filename.clone())
        .unwrap_or_else(|| format!("{project}.jar"));
    Ok(Some(DeclaredMod {
        filename,
        default_enabled: true,
        source: SourceDecl::Modrinth {
            project_id: project,
            version_id: v.id,
        },
        display: None,
        slug: None,
    }))
}

/// A Modrinth mod is already in the pack when its project id is declared -- keyed on
/// the project, not the filename, so a pulled dep is not re-added under a different
/// display name.
fn already_present(cfg: &PackConfig, decl: &DeclaredMod) -> bool {
    let SourceDecl::Modrinth { project_id, .. } = &decl.source else {
        return false;
    };
    cfg.mods
        .iter()
        .any(|m| matches!(&m.source, SourceDecl::Modrinth { project_id: p, .. } if p == project_id))
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
            pack_meta: Default::default(),
            owner: default_owner(),
            tier: default_tier(),
            visibility: default_visibility(),
            fork_of: None,
        }
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
