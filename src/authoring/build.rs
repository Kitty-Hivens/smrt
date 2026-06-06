//! The build pass: resolve every source in a `PackConfig` into the wire
//! `PackManifest`, and derive the `PackSummary` card. Pure compute -- reads
//! cache jars + Modrinth, writes nothing; the caller persists via `Storage`.

use super::curator::PackMeta;
use super::modrinth::Modrinth;
use super::sources::{ModrinthCache, resolve_asset, resolve_mod, sha1_hex};
use crate::domain::{
    AssetEntry, JavaSpec, LoaderSpec, MinecraftSpec, ModEntry, PackConfig, PackManifest,
    PackSummary, SCHEMA_VERSION,
};
use anyhow::{Result, bail};
use std::path::Path;
use tracing::info;

/// Resolve every source in a `PackConfig` and assemble the wire manifest.
/// Reads cache jars under `storage` and looks up Modrinth sources; does not
/// write anything. `pack_version` defaults to today's UTC `YYYY.MM.DD` slug.
pub async fn build_manifest(
    cfg: &PackConfig,
    storage: &Path,
    pack_version: Option<&str>,
    mirror_base: &str,
) -> Result<PackManifest> {
    let pack_version = pack_version.map(str::to_string).unwrap_or_else(today_slug);
    validate_canonical_pack_version(&pack_version)?;
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
        generated_at: now_rfc3339(),
        fingerprint: Some(fingerprint),
        minecraft,
        loader: cfg.loader.clone(),
        java,
        mods: mod_entries,
        assets: asset_entries,
    })
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
/// the config + the resolved `pack_version`, merging optional rich pack-meta
/// (icon / banner / gallery / description) on top.
pub fn make_pack_summary(
    cfg: &PackConfig,
    pack_version: &str,
    pack_meta: &PackMeta,
) -> PackSummary {
    PackSummary {
        pack_id: cfg.pack_id.clone(),
        display_name: cfg.display_name.clone(),
        tagline: cfg.tagline.clone(),
        minecraft_version: cfg.minecraft_version.clone(),
        latest_pack_version: pack_version.to_string(),
        tags: cfg.tags.clone(),
        featured: cfg.featured,
        icon_url: pack_meta.icon_url.clone(),
        banner_url: pack_meta.banner_url.clone(),
        gallery_urls: pack_meta.gallery_urls.clone(),
        description_md: pack_meta.description_md.clone(),
    }
}

fn now_rfc3339() -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Enforce the spec's canonical-form rule for `pack_version`: no trailing
/// `.0` segments. Equivalent strings under the comparator must also be
/// byte-equal so clients can use string equality for "did the latest version
/// change?" without re-running the comparator.
fn validate_canonical_pack_version(v: &str) -> Result<()> {
    if v.is_empty() {
        bail!("pack_version must not be empty");
    }
    let segments: Vec<&str> = v.split('.').collect();
    for seg in &segments {
        if seg.is_empty() || !seg.chars().all(|c| c.is_ascii_digit()) {
            bail!("pack_version segment {seg:?} is not a positive integer");
        }
    }
    if segments.last().is_some_and(|s| *s == "0") && segments.len() > 1 {
        bail!(
            "pack_version {v} is not canonical: trailing .0 segments are forbidden \
             (drop the trailing zero, e.g. write 2026.05.22 instead of 2026.05.22.0)"
        );
    }
    Ok(())
}

fn today_slug() -> String {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}.{:02}.{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_pack_version_accepts_typical_forms() {
        validate_canonical_pack_version("2026.05.22").unwrap();
        validate_canonical_pack_version("2026.05.22.1").unwrap();
        validate_canonical_pack_version("2026.05.22.10").unwrap();
    }

    #[test]
    fn canonical_pack_version_rejects_trailing_zero() {
        assert!(validate_canonical_pack_version("2026.05.22.0").is_err());
        assert!(validate_canonical_pack_version("2026.05.22.1.0").is_err());
    }

    #[test]
    fn canonical_pack_version_rejects_non_numeric() {
        assert!(validate_canonical_pack_version("2026.05.22a").is_err());
        assert!(validate_canonical_pack_version("v1").is_err());
        assert!(validate_canonical_pack_version("").is_err());
    }

    use crate::domain::Source;

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
