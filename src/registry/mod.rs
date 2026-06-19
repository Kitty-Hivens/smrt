//! Relational mod-identity registry (embedded SQLite). The mirror's index of
//! mods by identity (not filename): which packs use a mod, all versions of a
//! mod, orphan jars, loader-target eligibility, and sourced dep/conflict facts.
//!
//! Jar blobs stay content-addressed on the FS cache; this holds metadata +
//! relations. Every fact row carries a `source` so harvested rows (rebuildable
//! from jars + Modrinth) stay distinct from authored rows (manual moderation,
//! Phase 2) and a re-harvest never clobbers the authored ones.

pub(crate) mod authored;
mod db;
mod migrations;
pub mod model;
pub mod queries;
pub(crate) mod upsert;

pub use db::Registry;

#[cfg(test)]
mod tests {
    use super::model::{RelKind, Source};
    use super::{Registry, queries, upsert};

    const NOW: &str = "2026-06-06T00:00:00Z";

    // A registry with two forge mods (one referenced by a build, one orphan),
    // an `any` tweaker, a cleanroom-only artifact, a forge+fabric multi-loader
    // jar, a pack build, and a relation.
    fn fixture() -> Registry {
        let r = Registry::open_in_memory().unwrap();
        r.with_conn_mut(|c| {
            // appleskin: carries both a modid and a Modrinth project id
            let apple = upsert::upsert_mod_by_alias(
                c,
                &[("modid", "appleskin"), ("modrinth", "EsAfb37o")],
                NOW,
            )?;
            let apple_v = upsert::upsert_mod_version(
                c,
                apple,
                "2.5.1",
                &["forge"],
                "sha_apple",
                1000,
                Some("appleskin.jar"),
                None,
                NOW,
            )?;
            // jei: forge, modid only
            let jei = upsert::upsert_mod_by_alias(c, &[("modid", "jei")], NOW)?;
            let jei_v = upsert::upsert_mod_version(
                c,
                jei,
                "4.16",
                &["forge"],
                "sha_jei",
                2000,
                Some("jei.jar"),
                None,
                NOW,
            )?;
            // a loader-agnostic tweaker
            let tw = upsert::upsert_mod_by_alias(c, &[("modid", "tweak")], NOW)?;
            let tw_v = upsert::upsert_mod_version(
                c,
                tw,
                "1.0",
                &["any"],
                "sha_tweak",
                300,
                Some("tweak.jar"),
                None,
                NOW,
            )?;
            // a cleanroom-only artifact (not in any build)
            let cr = upsert::upsert_mod_by_alias(c, &[("modid", "crmod")], NOW)?;
            upsert::upsert_mod_version(
                c,
                cr,
                "1.0",
                &["cleanroom"],
                "sha_cr",
                400,
                Some("cr.jar"),
                None,
                NOW,
            )?;
            // a single jar published for two loaders at once (Modrinth set)
            let multi = upsert::upsert_mod_by_alias(c, &[("modid", "multimod")], NOW)?;
            upsert::upsert_mod_version(
                c,
                multi,
                "3.0",
                &["forge", "fabric"],
                "sha_multi",
                600,
                Some("multi.jar"),
                None,
                NOW,
            )?;
            // an orphan forge artifact (cached, no build references it)
            let orph = upsert::upsert_mod_by_alias(c, &[("modid", "orphanmod")], NOW)?;
            upsert::upsert_mod_version(
                c,
                orph,
                "0.1",
                &["forge"],
                "sha_orphan",
                50,
                Some("orphan.jar"),
                None,
                NOW,
            )?;
            // a pack build shipping appleskin + jei + tweak
            upsert::upsert_pack(c, "Industrial", "sc", NOW)?;
            let build = upsert::upsert_pack_build(
                c,
                "Industrial",
                "2026.06.06",
                "1.12.2",
                Some("forge"),
                Some("14.23"),
                Some(8),
                Some("fp_industrial"),
                true,
                NOW,
            )?;
            upsert::link_build_mod(c, build, apple_v, "appleskin.jar", true, true)?;
            upsert::link_build_mod(c, build, jei_v, "jei.jar", true, true)?;
            upsert::link_build_mod(c, build, tw_v, "tweak.jar", false, true)?;
            // jei requires appleskin (jar-meta)
            upsert::upsert_relation(
                c,
                jei,
                "appleskin",
                None,
                RelKind::Requires,
                Source::JarMeta,
                NOW,
            )?;
            Ok(())
        })
        .unwrap();
        r
    }

    #[test]
    fn alias_collapse_one_identity() {
        let r = fixture();
        r.with_conn(|c| {
            let by_modid = queries::mod_id_for_alias(c, "modid", "appleskin")?.unwrap();
            let by_project = queries::mod_id_for_alias(c, "modrinth", "EsAfb37o")?.unwrap();
            assert_eq!(
                by_modid, by_project,
                "modid + project_id collapse to one mod"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn q1_packs_using_mod() {
        let r = fixture();
        r.with_conn(|c| {
            let uses = queries::packs_using_mod(c, "modid", "appleskin")?;
            assert_eq!(uses.len(), 1);
            assert_eq!(uses[0].pack_id, "Industrial");
            assert_eq!(uses[0].filename, "appleskin.jar");
            // reachable via the Modrinth alias too
            assert_eq!(
                queries::packs_using_mod(c, "modrinth", "EsAfb37o")?.len(),
                1
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn q2_orphans_only_unreferenced() {
        let r = fixture();
        r.with_conn(|c| {
            let orphans = queries::orphan_jars(c)?;
            let shas: Vec<_> = orphans.iter().map(|o| o.sha1.as_str()).collect();
            assert!(shas.contains(&"sha_orphan"));
            assert!(shas.contains(&"sha_cr")); // cleanroom artifact is in no build
            assert!(!shas.contains(&"sha_apple"));
            assert!(!shas.contains(&"sha_jei"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn q3_versions_of_mod() {
        let r = fixture();
        r.with_conn(|c| {
            let vs = queries::versions_of_mod(c, "modid", "appleskin")?;
            assert_eq!(vs.len(), 1);
            assert_eq!(vs[0].version, "2.5.1");
            assert_eq!(vs[0].targets, vec!["forge".to_string()]);
            // the multi-loader jar folds both targets into one version row
            let ms = queries::versions_of_mod(c, "modid", "multimod")?;
            assert_eq!(ms.len(), 1);
            let mut t = ms[0].targets.clone();
            t.sort();
            assert_eq!(t, vec!["fabric".to_string(), "forge".to_string()]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn q4_family_reachability_is_directional() {
        let r = fixture();
        r.with_conn(|c| {
            let eligible = |loader: &str| -> Vec<String> {
                queries::eligible_for_loader(c, loader)
                    .unwrap()
                    .into_iter()
                    .map(|e| e.sha1)
                    .collect()
            };
            // cleanroom inherits forge -> forge + cleanroom + any + the multi jar
            let cr = eligible("cleanroom");
            for s in ["sha_apple", "sha_tweak", "sha_cr", "sha_multi"] {
                assert!(cr.contains(&s.to_string()), "cleanroom should see {s}");
            }
            // forge build does NOT pick up the cleanroom-only artifact, but does
            // see the forge+fabric jar (eligibility is per-target)
            let fo = eligible("forge");
            assert!(fo.contains(&"sha_apple".to_string()));
            assert!(fo.contains(&"sha_tweak".to_string())); // any
            assert!(fo.contains(&"sha_multi".to_string())); // forge of forge+fabric
            assert!(!fo.contains(&"sha_cr".to_string()));
            // fabric sees the multi jar via its fabric target + the any tweaker,
            // and none of the forge-only artifacts
            let fa = eligible("fabric");
            assert!(fa.contains(&"sha_multi".to_string()));
            assert!(fa.contains(&"sha_tweak".to_string()));
            assert!(!fa.contains(&"sha_apple".to_string()));
            assert!(!fa.contains(&"sha_cr".to_string()));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn never_clobber_authored_version() {
        let r = fixture();
        r.with_conn_mut(|c| {
            // promote jei's artifact to an authored row with a hand-set version
            c.execute(
                "UPDATE mod_version SET source = 'authored', version = 'AUTH' WHERE sha1 = 'sha_jei'",
                [],
            )?;
            // a re-harvest of the same sha1 must not overwrite it
            let jei = queries::mod_id_for_alias(c, "modid", "jei")?.unwrap();
            upsert::upsert_mod_version(
                c, jei, "9.9.9", &["forge"], "sha_jei", 2000, Some("jei.jar"), None, NOW,
            )?;
            let v: String =
                c.query_row("SELECT version FROM mod_version WHERE sha1 = 'sha_jei'", [], |r| {
                    r.get(0)
                })?;
            assert_eq!(v, "AUTH", "authored row left untouched by re-harvest");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn browser_list_mods_filters() {
        let r = fixture();
        r.with_conn(|c| {
            // unfiltered: every harvested mod, named by its modid (no canonical
            // name set in the fixture)
            let all = queries::list_mods(c, None, None, None)?;
            let names: Vec<_> = all.iter().map(|m| m.name.as_str()).collect();
            assert!(names.contains(&"appleskin"));
            assert!(names.contains(&"multimod"));
            assert_eq!(all.len(), 6);
            // a loader filter keeps `any` + that loader's artifacts
            let fabric = queries::list_mods(c, None, Some("fabric"), None)?;
            let fnames: Vec<_> = fabric.iter().map(|m| m.name.as_str()).collect();
            assert!(fnames.contains(&"multimod")); // forge+fabric jar
            assert!(fnames.contains(&"tweak")); // any
            assert!(!fnames.contains(&"appleskin")); // forge-only
            // a name query matches the modid alias
            let apple = queries::list_mods(c, Some("apple"), None, None)?;
            assert_eq!(apple.len(), 1);
            assert_eq!(apple[0].name, "appleskin");
            // loader filter is case-insensitive (pack loader "Forge" -> "forge")
            let upper = queries::list_mods(c, None, Some("Forge"), None)?;
            assert!(upper.iter().any(|m| m.name == "appleskin"));
            // the multi-loader jar reports both loaders as facets
            let multi = all.iter().find(|m| m.name == "multimod").unwrap();
            assert_eq!(multi.loaders, vec!["fabric".to_string(), "forge".to_string()]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn browser_list_builds_and_build_mods() {
        let r = fixture();
        r.with_conn(|c| {
            let builds = queries::list_builds(c)?;
            assert_eq!(builds.len(), 1);
            assert_eq!(builds[0].pack_id, "Industrial");
            assert_eq!(builds[0].mod_count, 3);
            assert!(builds[0].is_latest);

            let mods = queries::build_mods(c, "Industrial", "2026.06.06")?;
            assert_eq!(mods.len(), 3);
            let names: Vec<_> = mods.iter().map(|m| m.name.as_str()).collect();
            assert!(names.contains(&"appleskin"));
            assert!(names.contains(&"jei"));
            assert!(names.contains(&"tweak"));
            // each row resolves to the artifact sha1 the operator would re-add
            assert!(mods.iter().all(|m| !m.sha1.is_empty()));
            // tweak ships as optional in the fixture build
            let tweak = mods.iter().find(|m| m.name == "tweak").unwrap();
            assert!(!tweak.required);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn browser_mc_facet_and_filter() {
        // a dedicated registry so we control mc_versions (the shared fixture
        // leaves them unset)
        let r = Registry::open_in_memory().unwrap();
        r.with_conn_mut(|c| {
            let m = upsert::upsert_mod_by_alias(c, &[("modid", "biomesoplenty")], NOW)?;
            upsert::upsert_mod_version(
                c,
                m,
                "7.0",
                &["forge"],
                "sha_bop",
                1000,
                Some("bop.jar"),
                Some(r#"["1.12.2"]"#),
                NOW,
            )?;
            Ok(())
        })
        .unwrap();
        r.with_conn(|c| {
            // the mc set folds into both the summary facet and the version row
            let hit = queries::list_mods(c, None, None, Some("1.12.2"))?;
            assert_eq!(hit.len(), 1);
            assert_eq!(hit[0].mc_versions, vec!["1.12.2".to_string()]);
            assert!(queries::list_mods(c, None, None, Some("1.20.1"))?.is_empty());
            let vs = queries::versions_of_mod(c, "modid", "biomesoplenty")?;
            assert_eq!(vs[0].mc_versions, vec!["1.12.2".to_string()]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn browser_name_query_treats_underscore_literally() {
        // an unescaped LIKE would let `_` wildcard-match any char; with ESCAPE the
        // query 'iron_chests' must match only the literal-underscore mod, not the
        // sibling where '_' would stand in for 'x'
        let r = Registry::open_in_memory().unwrap();
        r.with_conn_mut(|c| {
            for (modid, sha) in [("iron_chests", "sha_ic"), ("ironxchests", "sha_ix")] {
                let m = upsert::upsert_mod_by_alias(c, &[("modid", modid)], NOW)?;
                upsert::upsert_mod_version(c, m, "1", &["forge"], sha, 1, None, None, NOW)?;
            }
            Ok(())
        })
        .unwrap();
        r.with_conn(|c| {
            let hits = queries::list_mods(c, Some("iron_chests"), None, None)?;
            assert_eq!(hits.len(), 1, "underscore matched literally, not as a wildcard");
            assert_eq!(hits[0].name, "iron_chests");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn relation_dedupe_on_reinsert() {
        let r = fixture();
        r.with_conn_mut(|c| {
            let jei = queries::mod_id_for_alias(c, "modid", "jei")?.unwrap();
            // same sourced assertion again -> ignored
            upsert::upsert_relation(c, jei, "appleskin", None, RelKind::Requires, Source::JarMeta, NOW)?;
            let n: i64 = c.query_row(
                "SELECT count(*) FROM relation WHERE from_mod_id = ?1 AND target_modid = 'appleskin'",
                [jei],
                |r| r.get(0),
            )?;
            assert_eq!(n, 1);
            Ok(())
        })
        .unwrap();
    }
}
