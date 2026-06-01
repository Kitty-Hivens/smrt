//! The bootstrap pass: read an SC archive, stage its mods into the cache +
//! extras into the pack's static area, and return a starter `PackConfig`.
//! Modrinth matches become modrinth sources; the rest become smrt_cache /
//! smrt_static. The caller persists the returned config.

use super::archive::{extract_extra_assets, extract_mods};
use super::modrinth::Modrinth;
use super::sources::{write_to_cache, write_to_static};
use crate::domain::{DeclaredAsset, DeclaredMod, LoaderSpec, PackConfig, SourceDecl};
use anyhow::{Result, bail};
use std::path::PathBuf;
use tracing::info;

pub struct BootstrapArgs {
    pub pack_id: String,
    pub display_name: String,
    pub tagline: String,
    pub minecraft_version: String,
    pub loader: LoaderSpec,
    pub java_major: u32,
    /// Storage root: extracted mod jars land in `{storage}/cache/`, extras
    /// land in `{storage}/packs/{pack_id}/static/`.
    pub storage: PathBuf,
}

pub async fn bootstrap(args: BootstrapArgs, archive_bytes: &[u8]) -> Result<PackConfig> {
    info!(bytes = archive_bytes.len(), "loaded SC archive");

    let mods = extract_mods(archive_bytes)?;
    info!(count = mods.len(), "discovered mods in archive");
    if mods.is_empty() {
        bail!("no mods/*.jar in archive -- wrong archive layout?");
    }

    let extras = extract_extra_assets(archive_bytes)?;
    info!(count = extras.len(), "discovered extras files");

    let modrinth = Modrinth::new()?;
    let sha1s: Vec<String> = mods.iter().map(|m| m.sha1.clone()).collect();
    let hits = modrinth.version_files_by_sha1(&sha1s).await?;
    info!(matched = hits.len(), total = sha1s.len(), "modrinth lookup");

    let cache_dir = args.storage.join("cache");
    let static_dir = args
        .storage
        .join("packs")
        .join(&args.pack_id)
        .join("static");

    let mut declared_mods = Vec::with_capacity(mods.len());
    for m in &mods {
        let decl = if let Some(hit) = hits.get(&m.sha1) {
            let mc_ok = hit
                .game_versions
                .iter()
                .any(|v| v == &args.minecraft_version);
            let loader_ok = hit
                .loaders
                .iter()
                .any(|l| l.eq_ignore_ascii_case(&args.loader.name));
            if mc_ok && loader_ok {
                DeclaredMod {
                    filename: m.filename.clone(),
                    required: true,
                    default_enabled: true,
                    source: SourceDecl::Modrinth {
                        project_id: hit.project_id.clone(),
                        version_id: hit.id.clone(),
                    },
                    display: None,
                    note: Some(format!("matched on Modrinth ({})", hit.version_number)),
                }
            } else {
                write_to_cache(&cache_dir, &m.sha1, &m.bytes)?;
                DeclaredMod {
                    filename: m.filename.clone(),
                    required: true,
                    default_enabled: true,
                    source: SourceDecl::SmrtCache {
                        sha1: m.sha1.clone(),
                    },
                    display: None,
                    note: Some(format!(
                        "TODO: Modrinth hit exists but mc/loader mismatch (mc={:?}, loaders={:?}); review for substitution",
                        hit.game_versions, hit.loaders
                    )),
                }
            }
        } else {
            write_to_cache(&cache_dir, &m.sha1, &m.bytes)?;
            DeclaredMod {
                filename: m.filename.clone(),
                required: true,
                default_enabled: true,
                source: SourceDecl::SmrtCache {
                    sha1: m.sha1.clone(),
                },
                display: None,
                note: Some(
                    "TODO: no Modrinth match; check if a relabel of an upstream project".into(),
                ),
            }
        };
        declared_mods.push(decl);
    }
    declared_mods.sort_by(|a, b| a.filename.cmp(&b.filename));

    let mut declared_assets = Vec::with_capacity(extras.len());
    for a in &extras {
        write_to_static(&static_dir, &a.rel_path, &a.bytes)?;
        declared_assets.push(DeclaredAsset {
            dest: a.rel_path.clone(),
            required: true,
            source: SourceDecl::SmrtStatic {
                rel_path: a.rel_path.clone(),
            },
            display: None,
            note: Some("TODO: review whether to keep SC default or curate replacement".into()),
        });
    }
    declared_assets.sort_by(|a, b| a.dest.cmp(&b.dest));

    Ok(PackConfig {
        pack_id: args.pack_id,
        display_name: args.display_name,
        tagline: args.tagline,
        minecraft_version: args.minecraft_version,
        loader: args.loader,
        java_major: args.java_major,
        tags: Vec::new(),
        featured: false,
        mods: declared_mods,
        assets: declared_assets,
    })
}
