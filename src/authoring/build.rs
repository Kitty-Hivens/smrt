//! The build pass: resolve every source in a `PackConfig` into the wire
//! `PackManifest`, and derive the `PackSummary` card. Pure compute -- reads
//! cache jars + Modrinth, writes nothing; the caller persists via `Storage`.

use super::modrinth::Modrinth;
use super::sources::{ModrinthCache, resolve_asset, resolve_mod, sha1_hex};
use crate::domain::{
    AssetEntry, Display, JavaSpec, LoaderSpec, MatchPolicy, MinecraftSpec, ModEntry, PackConfig,
    PackManifest, PackSummary, PresenceClass, SCHEMA_VERSION, SideClass, VersionChannel,
};
use crate::registry::classify::Classification;
use anyhow::{Result, bail};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::info;

/// Resolve every source in a `PackConfig` and assemble the wire manifest.
/// Reads cache jars under `storage` and looks up Modrinth sources; does not
/// write anything. `pack_version` defaults to the next auto-numbered
/// `<base>.<counter>` (see `resolve_auto_version`); `channel` is stored on the
/// manifest verbatim -- the version string carries no channel semantics;
/// `changelog` is the curator's release notes, stored as given.
/// `classifications` is the pack's side/policy map (`resolve::classify_pack`),
/// keyed by filename; an absent entry reads as unclassified.
pub async fn build_manifest(
    cfg: &PackConfig,
    storage: &Path,
    pack_version: Option<&str>,
    channel: VersionChannel,
    changelog: Option<String>,
    mirror_base: &str,
    classifications: &HashMap<String, Classification>,
) -> Result<PackManifest> {
    let pack_version = match pack_version {
        Some(v) => {
            validate_pack_version(v)?;
            v.to_string()
        }
        None => resolve_auto_version(cfg, storage).await?,
    };
    info!(
        pack_id = %cfg.pack_id,
        pack_version = %pack_version,
        mods = cfg.mods.len(),
        assets = cfg.assets.len(),
        "building manifest"
    );

    let modrinth = Modrinth::new()?;
    let modrinth_cache = ModrinthCache::default();

    let mut mod_entries = Vec::with_capacity(cfg.mods.len());
    for m in &cfg.mods {
        mod_entries.push(resolve_mod(m, storage, mirror_base, &modrinth, &modrinth_cache).await?);
    }
    mod_entries.sort_by(|a, b| a.filename.cmp(&b.filename));
    derive_required(&mut mod_entries, classifications)?;

    let mut asset_entries = Vec::with_capacity(cfg.assets.len());
    for a in &cfg.assets {
        asset_entries.push(
            resolve_asset(
                a,
                &cfg.pack_id,
                storage,
                mirror_base,
                &modrinth,
                &modrinth_cache,
            )
            .await?,
        );
    }
    asset_entries.sort_by(|a, b| a.dest.cmp(&b.dest));

    let minecraft = MinecraftSpec {
        version: cfg.minecraft_version.clone(),
    };
    let java = JavaSpec {
        major: cfg.java_major,
    };
    let fingerprint =
        content_fingerprint(&minecraft, &cfg.loader, &java, &mod_entries, &asset_entries);

    Ok(PackManifest {
        schema_version: SCHEMA_VERSION,
        pack_id: cfg.pack_id.clone(),
        pack_version,
        channel: Some(channel),
        changelog,
        generated_at: now_rfc3339(),
        fingerprint: Some(fingerprint),
        minecraft,
        loader: cfg.loader.clone(),
        java,
        mods: mod_entries,
        assets: asset_entries,
    })
}

/// Derive each entry's required-ness from the side/policy classification plus
/// the dependency graph, never a hand-set flag:
///
///   required = { default-enabled must_match mods }
///            + their transitive hard dependencies
///            + the transitive hard dependencies of every default-enabled mod
///
/// A default-enabled must_match mod (a content mod: the server carries it, so
/// the client must too) is required in itself -- the top-level-content fix. An
/// opted-out must_match mod stays out: the curator removed it from the default
/// server set, and forcing it back would erase the opt-out.
///
/// Side invariants: a server-side mod is never required for the client and
/// ships opted out (advisory in the resolve report, nothing is removed); a
/// coremod/library jar is never required (always toggleable); a confidently
/// client-side mod never locks through the graph at all (client chains
/// co-toggle in the launcher via the requires tree), with the low-confidence
/// declared-edge override as the one exception and a build error as the
/// backstop for an inconsistent classification.
fn derive_required(
    mods: &mut [ModEntry],
    classifications: &HashMap<String, Classification>,
) -> Result<()> {
    let idx: HashMap<String, usize> = mods
        .iter()
        .enumerate()
        .map(|(i, m)| (m.filename.clone(), i))
        .collect();
    let side = |m: &ModEntry| -> Option<SideClass> {
        classifications.get(&m.filename).and_then(|c| c.side)
    };
    let non_mod = |m: &ModEntry| -> bool {
        classifications
            .get(&m.filename)
            .is_some_and(|c| c.is_non_mod())
    };

    // Server-side mods leave the default install before anything is seeded, so
    // their own dependencies are not pulled in on their account.
    for m in mods.iter_mut() {
        if side(m) == Some(SideClass::Server) {
            m.default_enabled = false;
        }
    }

    // A hard edge into a confidently client-side mod never contributes to the
    // required walk: locking it is exactly what the client invariant forbids,
    // and a client mod hard-requiring another client mod (EMF -> ETF) is a
    // legitimate client-internal chain the launcher co-toggles through the
    // requires tree, not a reason to force-install the target on everyone.
    // The one exception stays: a LOW-confidence client verdict (the surface
    // heuristic) yields to a declared edge -- the bspkrsCore-class library
    // shape -- so those targets do lock, with a warning below.
    let lockable = |i: usize| -> bool {
        match classifications.get(&mods[i].filename) {
            Some(c) => c.side != Some(SideClass::Client) || c.client_verdict_is_soft(),
            None => true,
        }
    };
    let hard_deps = |m: &ModEntry| -> Vec<usize> {
        m.display
            .as_ref()
            .map(|d| {
                d.requires
                    .iter()
                    .filter(|r| !r.optional)
                    .filter_map(|r| idx.get(&r.filename).copied())
                    .filter(|i| lockable(*i))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut required: HashSet<usize> = mods
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.default_enabled
                && !non_mod(m)
                && classifications
                    .get(&m.filename)
                    .is_some_and(|c| c.policy == Some(MatchPolicy::MustMatch))
        })
        .map(|(i, _)| i)
        .collect();
    let mut queue: Vec<usize> = mods
        .iter()
        .enumerate()
        .filter(|(i, m)| m.default_enabled || required.contains(i))
        .flat_map(|(_, m)| hard_deps(m))
        .collect();
    while let Some(i) = queue.pop() {
        if required.insert(i) {
            queue.extend(hard_deps(&mods[i]));
        }
    }

    for (i, m) in mods.iter().enumerate() {
        if !required.contains(&i) {
            continue;
        }
        // never required: server-side mods (not the client's problem) and
        // not-a-mod jars (always toggleable)
        if side(m) == Some(SideClass::Server) || non_mod(m) {
            required.remove(&i);
            continue;
        }
        // The client invariant, as a backstop. Confidently-client targets are
        // excluded from the walk above and a client mod cannot be a must_match
        // seed (the env mapping never yields that pair and the authored writer
        // rejects it), so a required client survivor here is either the
        // deliberate low-confidence override -- a declared edge outweighing
        // the surface heuristic (bspkrsCore-class) -- or an inconsistency this
        // refuses to ship.
        if side(m) == Some(SideClass::Client) {
            if classifications
                .get(&m.filename)
                .is_some_and(|c| c.client_verdict_is_soft())
            {
                tracing::warn!(
                    filename = %m.filename,
                    "declared hard edges outweigh a low-confidence client verdict; locking required"
                );
                continue;
            }
            bail!(
                "client-side mod {} would be locked required -- a client mod is never force-installed; fix the mod's classification",
                m.filename
            );
        }
    }

    for (i, m) in mods.iter_mut().enumerate() {
        m.required = required.contains(&i);
        // The advisory presence class, collapsing side + policy + the graph
        // outcome for the launcher UI. Unclassified stays absent -- an old-style
        // entry -- and a stale value from a previous build never lingers.
        let presence = match classifications.get(&m.filename) {
            Some(c) if c.is_non_mod() => Some(PresenceClass::Coremod),
            Some(c) => match c.side {
                // a required client survivor exists only via the soft-verdict
                // override; it reads required, not client
                Some(SideClass::Client) if !m.required => Some(PresenceClass::OptionalClient),
                Some(SideClass::Server) => Some(PresenceClass::OptionalServer),
                _ if m.required => Some(PresenceClass::Required),
                // policy does not matter for an unlocked both-side mod: an
                // opted-out must_match mod (a content mod the curator removed
                // from the default set) is exactly as toggleable as a tolerant
                // one -- must_match only drives the required seeding above
                Some(SideClass::Both) => Some(PresenceClass::OptionalBoth),
                _ => None,
            },
            None if m.required => Some(PresenceClass::Required),
            None => None,
        };
        match presence {
            Some(p) => m.display.get_or_insert_with(Display::default).presence = Some(p),
            None => {
                if let Some(d) = &mut m.display {
                    d.presence = None;
                }
            }
        }
    }
    Ok(())
}

/// Content fingerprint of a build: a sha1 over exactly what lands in an
/// instance -- each artifact's hash + install flags, plus the loader / java /
/// MC baseline. Deliberately excludes `pack_version` (the label this makes
/// derivable), `generated_at` (a timestamp, not content), and the advisory
/// `display` metadata (a description edit does not change the instance). Lines
/// are sorted, so the result is independent of mod/asset ordering: identical
/// content yields an identical fingerprint across rebuilds, a changed set
/// yields a new one.
fn content_fingerprint(
    minecraft: &MinecraftSpec,
    loader: &LoaderSpec,
    java: &JavaSpec,
    mods: &[ModEntry],
    assets: &[AssetEntry],
) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(mods.len() + assets.len() + 3);
    lines.push(format!("mc\t{}", minecraft.version));
    lines.push(format!("loader\t{}\t{}", loader.name, loader.version));
    lines.push(format!("java\t{}", java.major));
    for m in mods {
        lines.push(format!(
            "mod\t{}\t{}\t{}\t{}",
            m.filename, m.sha1, m.required, m.default_enabled
        ));
    }
    for a in assets {
        lines.push(format!("asset\t{}\t{}\t{}", a.dest, a.sha1, a.required));
    }
    lines.sort();
    sha1_hex(lines.join("\n").as_bytes())
}

/// Derive the `PackSummary` (the Browse-list / PackDetail card payload) from
/// the config + the resolved `pack_version`, carrying the config's pack-card
/// metadata (icon / banner / gallery / description) onto the summary.
pub fn make_pack_summary(cfg: &PackConfig, pack_version: &str) -> PackSummary {
    PackSummary {
        pack_id: cfg.pack_id.clone(),
        display_name: cfg.display_name.clone(),
        tagline: cfg.tagline.clone(),
        minecraft_version: cfg.minecraft_version.clone(),
        latest_pack_version: pack_version.to_string(),
        tags: cfg.tags.clone(),
        featured: cfg.featured,
        icon_url: cfg.pack_meta.icon_url.clone(),
        banner_url: cfg.pack_meta.banner_url.clone(),
        gallery_urls: cfg.pack_meta.gallery_urls.clone(),
        description_md: cfg.pack_meta.description_md.clone(),
        owner: cfg.owner,
        tier: cfg.tier,
        visibility: cfg.visibility,
        fork_of: cfg.fork_of.clone(),
        // read-time derivations (from the latest manifest), never persisted
        latest_built_at: None,
        latest_channel: None,
    }
}

fn now_rfc3339() -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Resolve the automatic build version `<base>.<counter>`: the base is the
/// config's hand-bumped `version` line (`0.0` if unset), the counter is one
/// past the highest already published for that base, from zero. So a config at
/// base `0.4` builds `0.4.0`, `0.4.1`, ...; bumping the base to `0.5` restarts
/// at `0.5.0`. Plain numbers only -- the channel lives in its own manifest
/// field, and the build date in `generated_at`. Forward-only: existing
/// date/SNAPSHOT versions keep their strings; only new builds adopt this form.
async fn resolve_auto_version(cfg: &PackConfig, storage: &Path) -> Result<String> {
    let base = cfg.version.as_deref().unwrap_or("0.0");
    let existing = existing_versions(storage, &cfg.pack_id).await;
    let resolved = next_auto_version(base, &existing);
    validate_pack_version(&resolved)?;
    Ok(resolved)
}

/// `<base>.<N>` where N is one past the highest counter already published for
/// that exact base (from zero). Max, not first-free: a deleted build's number
/// is never reissued. Pure.
fn next_auto_version(base: &str, existing: &[String]) -> String {
    let next = existing
        .iter()
        .filter_map(|v| v.strip_prefix(base)?.strip_prefix('.')?.parse::<u64>().ok())
        .max()
        .map_or(0, |n| n + 1);
    format!("{base}.{next}")
}

/// Version strings already published for a pack (the manifest filenames, minus
/// the `latest` pointer). Best-effort: a missing pack dir yields none.
async fn existing_versions(storage: &Path, pack_id: &str) -> Vec<String> {
    let dir = storage.join("packs").join(pack_id).join("manifests");
    let mut out = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return out;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Some(name) = entry.file_name().to_str()
            && let Some(stem) = name.strip_suffix(".json")
            && stem != "latest"
        {
            out.push(stem.to_string());
        }
    }
    out
}

/// Validate a `pack_version`: the new `SNAPSHOT-<version>-<date>[.N]` form or a
/// legacy bare-numeric one (`2026.05.22[.N]`). Every segment is a positive
/// integer; a trailing `.0` is rejected so equal versions are byte-equal and a
/// client can compare the string directly for "did the latest version change?".
fn validate_pack_version(v: &str) -> Result<()> {
    if v.is_empty() {
        bail!("pack_version must not be empty");
    }
    let body = v.strip_prefix("SNAPSHOT-").unwrap_or(v);
    if body.is_empty() {
        bail!("pack_version {v:?} has an empty version body");
    }
    for seg in body.split(['-', '.']) {
        if seg.is_empty() || !seg.bytes().all(|b| b.is_ascii_digit()) {
            bail!("pack_version segment {seg:?} is not a positive integer");
        }
    }
    // A trailing .0 is legitimate under `<base>.<counter>` numbering (0.4.0 is
    // the first build of base 0.4); the old date scheme forbade it so tuple
    // equality implied string equality, which the counter scheme preserves by
    // never emitting a bare base as a version.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_version_accepts_legacy_and_snapshot_forms() {
        // legacy bare-numeric (the pre-rename versions stay valid)
        validate_pack_version("2026.05.22").unwrap();
        validate_pack_version("2026.05.22.1").unwrap();
        // new channel-prefixed form
        validate_pack_version("SNAPSHOT-0.0.0-2026.06.07").unwrap();
        validate_pack_version("SNAPSHOT-0.0.10-2026.06.07.1").unwrap();
        validate_pack_version("SNAPSHOT-1.2.3-2026.06.07.12").unwrap();
    }

    #[test]
    fn pack_version_accepts_a_zero_counter() {
        // `<base>.<counter>` numbering makes .0 the first build of a base
        assert!(validate_pack_version("0.4.0").is_ok());
        assert!(validate_pack_version("2026.05.22.0").is_ok());
    }

    #[test]
    fn pack_version_rejects_non_numeric_body() {
        assert!(validate_pack_version("2026.05.22a").is_err());
        assert!(validate_pack_version("v1").is_err());
        assert!(validate_pack_version("").is_err());
        assert!(validate_pack_version("SNAPSHOT-0.0.x-2026.06.07").is_err());
    }

    #[test]
    fn next_auto_version_counts_past_the_highest_and_ignores_legacy() {
        // fresh base -> counter starts at zero
        assert_eq!(next_auto_version("0.0", &[]), "0.0.0");
        // max + 1, never a gap re-fill: a deleted number is not reissued
        let existing = vec!["0.0.0".to_string(), "0.0.4".to_string()];
        assert_eq!(next_auto_version("0.0", &existing), "0.0.5");
        // bumping the base restarts the counter
        assert_eq!(next_auto_version("0.1", &existing), "0.1.0");
        // legacy date/SNAPSHOT strings and lookalike bases do not count
        let mixed = vec![
            "2026.05.22.2".to_string(),
            "SNAPSHOT-0.0.0-2026.07.18".to_string(),
            "0.40.1".to_string(),
            "0.4.2.1".to_string(),
            "0.4.7".to_string(),
        ];
        assert_eq!(next_auto_version("0.4", &mixed), "0.4.8");
    }

    use crate::domain::{Display, Requirement, Source};
    use crate::registry::classify::Provenance;

    fn cls(side: Option<SideClass>, policy: Option<MatchPolicy>) -> Classification {
        Classification {
            side,
            policy,
            kind: Some("mod".into()),
            provenance: Provenance::Bytecode,
            bytecode_side: side,
            bytecode_policy: policy,
            side_confidence: side.map(|_| "high".to_string()),
        }
    }

    fn entry(filename: &str, default_enabled: bool, requires: &[&str]) -> ModEntry {
        let display = (!requires.is_empty()).then(|| Display {
            requires: requires
                .iter()
                .map(|f| Requirement {
                    filename: f.to_string(),
                    version_range: None,
                    optional: false,
                })
                .collect(),
            ..Display::default()
        });
        ModEntry {
            filename: filename.into(),
            sha1: filename.into(),
            size_bytes: 1,
            required: false,
            default_enabled,
            source: Source::SmrtCache { url: "u".into() },
            display,
            slug: None,
        }
    }

    fn required_set(mods: &[ModEntry]) -> Vec<&str> {
        mods.iter()
            .filter(|m| m.required)
            .map(|m| m.filename.as_str())
            .collect()
    }

    // The symptom-1 fix: a default-enabled content mod (must_match) is required
    // in itself, with no dependents needed; a tolerant top-level mod stays
    // toggleable; the transitive hard deps of enabled mods stay locked. The
    // presence class rides out on the display block.
    #[test]
    fn must_match_content_mod_is_required_without_dependents() {
        let mut mods = vec![
            entry("ArsNouveau.jar", true, &[]),
            entry("JEI.jar", true, &[]),
            entry("addon.jar", true, &["lib.jar"]),
            entry("lib.jar", true, &[]),
        ];
        let cl = HashMap::from([
            (
                "ArsNouveau.jar".to_string(),
                cls(Some(SideClass::Both), Some(MatchPolicy::MustMatch)),
            ),
            (
                "JEI.jar".to_string(),
                cls(Some(SideClass::Both), Some(MatchPolicy::Tolerant)),
            ),
        ]);
        derive_required(&mut mods, &cl).unwrap();
        assert_eq!(required_set(&mods), vec!["ArsNouveau.jar", "lib.jar"]);

        let presence = |f: &str| {
            mods.iter()
                .find(|m| m.filename == f)
                .and_then(|m| m.display.as_ref())
                .and_then(|d| d.presence)
        };
        assert_eq!(presence("ArsNouveau.jar"), Some(PresenceClass::Required));
        assert_eq!(presence("JEI.jar"), Some(PresenceClass::OptionalBoth));
        assert_eq!(
            presence("lib.jar"),
            Some(PresenceClass::Required),
            "graph-locked without a classification still reads required"
        );
        assert_eq!(presence("addon.jar"), None, "unclassified stays absent");
    }

    // An opted-out must_match mod stays out: the curator removed it from the
    // default server set, and forcing it back would erase the opt-out. Its
    // presence still reads optional_both -- policy only drives the seeding,
    // not the chip.
    #[test]
    fn opted_out_must_match_mod_stays_optional() {
        let mut mods = vec![entry("lunary.jar", false, &[])];
        let cl = HashMap::from([(
            "lunary.jar".to_string(),
            cls(Some(SideClass::Both), Some(MatchPolicy::MustMatch)),
        )]);
        derive_required(&mut mods, &cl).unwrap();
        assert!(!mods[0].required);
        assert!(!mods[0].default_enabled);
        assert_eq!(
            mods[0].display.as_ref().and_then(|d| d.presence),
            Some(PresenceClass::OptionalBoth),
            "an opted-out content mod is still a classified both-side toggle"
        );
    }

    // A server-side mod is never required for the client and ships opted out,
    // even when a hard edge pulls at it; nothing is removed from the manifest.
    #[test]
    fn server_side_mod_ships_opted_out_and_never_required() {
        let mut mods = vec![
            entry("needs-server-util.jar", true, &["chunky.jar"]),
            entry("chunky.jar", true, &[]),
        ];
        let cl = HashMap::from([(
            "chunky.jar".to_string(),
            cls(Some(SideClass::Server), Some(MatchPolicy::Tolerant)),
        )]);
        derive_required(&mut mods, &cl).unwrap();
        let chunky = &mods[1];
        assert!(!chunky.required, "a server-side mod is never required");
        assert!(!chunky.default_enabled, "it ships opted out");
        assert_eq!(mods.len(), 2, "nothing is removed");
    }

    // A hard edge into a confidently client-side mod never locks it: the
    // invariant holds structurally, the build succeeds, and the dependency
    // stays in the requires tree for the launcher to co-toggle (EMF -> ETF).
    #[test]
    fn client_target_edge_does_not_lock_and_the_build_succeeds() {
        let mut mods = vec![
            entry(
                "EntityModelFeatures.jar",
                true,
                &["EntityTextureFeatures.jar"],
            ),
            entry("EntityTextureFeatures.jar", true, &[]),
        ];
        let cl = HashMap::from([
            (
                "EntityModelFeatures.jar".to_string(),
                cls(Some(SideClass::Client), Some(MatchPolicy::Tolerant)),
            ),
            (
                "EntityTextureFeatures.jar".to_string(),
                cls(Some(SideClass::Client), Some(MatchPolicy::Tolerant)),
            ),
        ]);
        derive_required(&mut mods, &cl).unwrap();
        assert!(!mods[1].required, "the client target stays toggleable");
        let presence = mods[1].display.as_ref().and_then(|d| d.presence);
        assert_eq!(presence, Some(PresenceClass::OptionalClient));
        // the edge itself survives for the launcher's dependency tree
        assert_eq!(
            mods[0].display.as_ref().unwrap().requires[0].filename,
            "EntityTextureFeatures.jar"
        );
    }

    // The invariant backstop still refuses an inconsistent classification (a
    // client-side must_match seed cannot arise from the env mapping, and the
    // authored writer rejects the pair).
    #[test]
    fn client_must_match_inconsistency_fails_the_build() {
        let mut mods = vec![entry("weird.jar", true, &[])];
        let cl = HashMap::from([(
            "weird.jar".to_string(),
            cls(Some(SideClass::Client), Some(MatchPolicy::MustMatch)),
        )]);
        let err = derive_required(&mut mods, &cl).unwrap_err().to_string();
        assert!(err.contains("weird.jar"), "names the mod: {err}");
    }

    // A low-confidence (surface-heuristic) client verdict yields to a declared
    // hard edge: the mod locks required with a warning instead of failing the
    // build -- the bspkrsCore-class library shape.
    #[test]
    fn soft_client_verdict_yields_to_a_declared_edge() {
        let mut mods = vec![
            entry("TreeCapitator.jar", true, &["bspkrsCore.jar"]),
            entry("bspkrsCore.jar", true, &[]),
        ];
        let mut soft = cls(Some(SideClass::Client), Some(MatchPolicy::Tolerant));
        soft.side_confidence = Some("low".into());
        let cl = HashMap::from([("bspkrsCore.jar".to_string(), soft)]);
        derive_required(&mut mods, &cl).unwrap();
        assert!(
            mods[1].required,
            "the declared edge wins over the heuristic"
        );
        let presence = mods[1].display.as_ref().and_then(|d| d.presence);
        assert_eq!(
            presence,
            Some(PresenceClass::Required),
            "a required survivor reads required, not client"
        );
    }

    // A coremod/library jar is never required, even when an edge points at it.
    #[test]
    fn non_mod_jar_is_never_required() {
        let mut mods = vec![
            entry("ccl.jar", true, &["chickenasm.jar"]),
            entry("chickenasm.jar", true, &[]),
        ];
        let mut asm = cls(None, None);
        asm.kind = Some("library".into());
        let cl = HashMap::from([("chickenasm.jar".to_string(), asm)]);
        derive_required(&mut mods, &cl).unwrap();
        assert!(!mods[1].required, "a not-a-mod jar stays toggleable");
    }

    fn mc() -> MinecraftSpec {
        MinecraftSpec {
            version: "1.12.2".into(),
        }
    }
    fn forge() -> LoaderSpec {
        LoaderSpec {
            name: "forge".into(),
            version: "14.23.5.2922".into(),
        }
    }
    fn modentry(filename: &str, sha1: &str) -> ModEntry {
        ModEntry {
            filename: filename.into(),
            sha1: sha1.into(),
            size_bytes: 1,
            required: true,
            default_enabled: true,
            source: Source::SmrtCache { url: "u".into() },
            display: None,
            slug: None,
        }
    }

    #[test]
    fn fingerprint_is_stable_and_order_independent() {
        let a = [modentry("a.jar", "aaa"), modentry("b.jar", "bbb")];
        let b = [modentry("b.jar", "bbb"), modentry("a.jar", "aaa")];
        let fa = content_fingerprint(&mc(), &forge(), &JavaSpec { major: 8 }, &a, &[]);
        let fb = content_fingerprint(&mc(), &forge(), &JavaSpec { major: 8 }, &b, &[]);
        assert_eq!(
            fa, fb,
            "reordering the same content must not change the hash"
        );
        assert_eq!(fa.len(), 40, "sha1 hex");
    }

    #[test]
    fn fingerprint_changes_on_content_change() {
        let base = [modentry("a.jar", "aaa")];
        let f0 = content_fingerprint(&mc(), &forge(), &JavaSpec { major: 8 }, &base, &[]);

        // a different artifact hash
        let swapped = [modentry("a.jar", "ccc")];
        let f1 = content_fingerprint(&mc(), &forge(), &JavaSpec { major: 8 }, &swapped, &[]);
        assert_ne!(f0, f1, "a changed mod sha1 changes the fingerprint");

        // a loader migration (same MC, new loader) -- the heavy update case
        let cleanroom = LoaderSpec {
            name: "cleanroom".into(),
            version: "0.2".into(),
        };
        let f2 = content_fingerprint(&mc(), &cleanroom, &JavaSpec { major: 8 }, &base, &[]);
        assert_ne!(f0, f2, "a loader change changes the fingerprint");

        // an install-flag flip (optional default off) changes the instance
        let mut toggled = modentry("a.jar", "aaa");
        toggled.required = false;
        toggled.default_enabled = false;
        let f3 = content_fingerprint(&mc(), &forge(), &JavaSpec { major: 8 }, &[toggled], &[]);
        assert_ne!(f0, f3, "install flags are part of the instance identity");
    }
}
