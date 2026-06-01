//! SC archive extraction: pull `mods/*.jar` and the bundled `extra.zip`
//! tree out of an SC client archive. Used by bootstrap (staging) and
//! validate (cross-reference). Internal to the authoring layer.

use super::sources::sha1_hex;
use anyhow::{Context, Result};
use std::io::{Cursor, Read};
use tracing::warn;

pub(super) struct DiscoveredMod {
    pub(super) sha1: String,
    pub(super) filename: String,
    pub(super) bytes: Vec<u8>,
}

pub(super) struct DiscoveredAsset {
    pub(super) rel_path: String,
    pub(super) bytes: Vec<u8>,
}

pub(super) fn extract_mods(archive_bytes: &[u8]) -> Result<Vec<DiscoveredMod>> {
    let reader = Cursor::new(archive_bytes);
    let mut zip = zip::ZipArchive::new(reader).context("opening SC archive as zip")?;
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).context("reading zip entry")?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().to_string();
        let segments: Vec<&str> = name.split('/').collect();
        let is_mod = segments.first() == Some(&"mods")
            && name.ends_with(".jar")
            && segments.last().map(|s| !s.is_empty()).unwrap_or(false);
        if !is_mod {
            continue;
        }
        let filename = segments.last().unwrap().to_string();
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .with_context(|| format!("reading {name}"))?;
        let sha1 = sha1_hex(&bytes);
        out.push(DiscoveredMod {
            sha1,
            filename,
            bytes,
        });
    }
    Ok(out)
}

pub(super) fn extract_extra_assets(archive_bytes: &[u8]) -> Result<Vec<DiscoveredAsset>> {
    let reader = Cursor::new(archive_bytes);
    let mut zip = zip::ZipArchive::new(reader).context("opening SC archive as zip")?;
    let mut extra_zip_bytes = None;
    if let Ok(mut entry) = zip.by_name("extra.zip") {
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf).context("reading extra.zip")?;
        extra_zip_bytes = Some(buf);
    }
    let Some(bytes) = extra_zip_bytes else {
        return Ok(Vec::new());
    };

    let mut inner = zip::ZipArchive::new(Cursor::new(bytes)).context("opening extra.zip")?;
    let mut out = Vec::new();
    for i in 0..inner.len() {
        let mut entry = inner.by_index(i).context("reading extra.zip entry")?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().to_string();
        if name.contains("..") || name.starts_with('/') {
            warn!(path = %name, "skipping suspicious extra.zip entry");
            continue;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut buf)
            .with_context(|| format!("reading extra entry {name}"))?;
        out.push(DiscoveredAsset {
            rel_path: name,
            bytes: buf,
        });
    }
    Ok(out)
}
