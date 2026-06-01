//! The build pass: resolve every source in a `PackConfig` into the wire
//! `PackManifest`, and derive the `PackSummary` card. Pure compute -- reads
//! cache jars + Modrinth, writes nothing; the caller persists via `Storage`.

use super::curator::PackMeta;
use super::modrinth::Modrinth;
use super::sources::{ModrinthCache, resolve_asset, resolve_mod};
use crate::domain::{
    JavaSpec, MinecraftSpec, PackConfig, PackManifest, PackSummary, SCHEMA_VERSION,
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

    Ok(PackManifest {
        schema_version: SCHEMA_VERSION,
        pack_id: cfg.pack_id.clone(),
        pack_version,
        generated_at: now_rfc3339(),
        minecraft: MinecraftSpec {
            version: cfg.minecraft_version.clone(),
        },
        loader: cfg.loader.clone(),
        java: JavaSpec {
            major: cfg.java_major,
        },
        mods: mod_entries,
        assets: asset_entries,
    })
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
}
