//! Source resolution + artifact I/O. Turns a declared source into a wire
//! `ModEntry` / `AssetEntry` (Modrinth lookup or local cache read), and holds
//! the cache/static read-write-URL helpers the build and bootstrap passes
//! share. Internal to the authoring layer.

use super::modrinth::{Modrinth, Version as MrVersion};
use crate::domain::{AssetEntry, DeclaredAsset, DeclaredMod, ModEntry, Source, SourceDecl};
use anyhow::{Context, Result, anyhow, bail};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(super) struct ModrinthCache {
    inner: tokio::sync::Mutex<HashMap<(String, String), MrVersion>>,
}

impl ModrinthCache {
    pub(super) async fn get_or_fetch(
        &self,
        modrinth: &Modrinth,
        project_id: &str,
        version_id: &str,
    ) -> Result<MrVersion> {
        let key = (project_id.to_string(), version_id.to_string());
        if let Some(v) = self.inner.lock().await.get(&key) {
            return Ok(v.clone());
        }
        let v = modrinth.project_version(project_id, version_id).await?;
        self.inner.lock().await.insert(key, v.clone());
        Ok(v)
    }
}

pub(super) async fn resolve_mod(
    decl: &DeclaredMod,
    storage: &Path,
    mirror_base: &str,
    modrinth: &Modrinth,
    cache: &ModrinthCache,
) -> Result<ModEntry> {
    let (sha1, size_bytes, source) = match &decl.source {
        SourceDecl::Modrinth {
            project_id,
            version_id,
        } => {
            let v = cache
                .get_or_fetch(modrinth, project_id, version_id)
                .await
                .with_context(|| format!("resolving Modrinth mod {}", decl.filename))?;
            let f = v.primary_file().ok_or_else(|| {
                anyhow!("Modrinth version {project_id}/{version_id} has no files")
            })?;
            (
                f.hashes.sha1.clone(),
                f.size,
                Source::Modrinth {
                    project_id: project_id.clone(),
                    version_id: version_id.clone(),
                },
            )
        }
        SourceDecl::SmrtCache { sha1 } => {
            let path = cache_jar_path(storage, sha1)?;
            let meta = fs::metadata(&path).with_context(|| {
                format!(
                    "cache jar {} not found for mod {}",
                    path.display(),
                    decl.filename
                )
            })?;
            (
                sha1.clone(),
                meta.len(),
                Source::SmrtCache {
                    url: cache_url(mirror_base, sha1),
                },
            )
        }
        SourceDecl::SmrtStatic { .. } => {
            bail!(
                "mod {} uses smrt_static source -- mods must be modrinth or smrt_cache",
                decl.filename
            );
        }
    };

    Ok(ModEntry {
        filename: decl.filename.clone(),
        sha1,
        size_bytes,
        required: decl.required,
        default_enabled: decl.default_enabled,
        source,
        display: decl.display.clone(),
    })
}

pub(super) async fn resolve_asset(
    decl: &DeclaredAsset,
    pack_id: &str,
    storage: &Path,
    mirror_base: &str,
    modrinth: &Modrinth,
    cache: &ModrinthCache,
) -> Result<AssetEntry> {
    let (sha1, size_bytes, source) = match &decl.source {
        SourceDecl::Modrinth {
            project_id,
            version_id,
        } => {
            let v = cache
                .get_or_fetch(modrinth, project_id, version_id)
                .await
                .with_context(|| format!("resolving Modrinth asset {}", decl.dest))?;
            let f = v.primary_file().ok_or_else(|| {
                anyhow!("Modrinth version {project_id}/{version_id} has no files")
            })?;
            (
                f.hashes.sha1.clone(),
                f.size,
                Source::Modrinth {
                    project_id: project_id.clone(),
                    version_id: version_id.clone(),
                },
            )
        }
        SourceDecl::SmrtStatic { rel_path } => {
            let path = static_asset_path(storage, pack_id, rel_path)?;
            let bytes = fs::read(&path).with_context(|| {
                format!(
                    "static asset {} not found for {}",
                    path.display(),
                    decl.dest
                )
            })?;
            let size = bytes.len() as u64;
            let sha = sha1_hex(&bytes);
            (
                sha,
                size,
                Source::SmrtStatic {
                    url: static_url(mirror_base, pack_id, rel_path),
                },
            )
        }
        SourceDecl::SmrtCache { .. } => {
            bail!(
                "asset {} uses smrt_cache source -- assets must be modrinth or smrt_static",
                decl.dest
            );
        }
    };

    Ok(AssetEntry {
        dest: decl.dest.clone(),
        sha1,
        size_bytes,
        required: decl.required,
        source,
        display: decl.display.clone(),
    })
}

pub(super) fn write_to_cache(cache_dir: &Path, sha1: &str, bytes: &[u8]) -> Result<()> {
    if sha1.len() != 40 || !sha1.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid sha1: {sha1}");
    }
    let prefix = &sha1[..2];
    let dir = cache_dir.join(prefix);
    fs::create_dir_all(&dir).context("creating cache prefix dir")?;
    let path = dir.join(format!("{sha1}.jar"));
    if path.exists() {
        return Ok(());
    }
    let tmp = path.with_extension("jar.tmp");
    fs::write(&tmp, bytes).context("writing cache jar tmp")?;
    fs::rename(&tmp, &path).context("renaming cache jar")?;
    Ok(())
}

pub(super) fn write_to_static(static_dir: &Path, rel_path: &str, bytes: &[u8]) -> Result<()> {
    let path = static_dir.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating static parent dir")?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).context("writing static tmp")?;
    fs::rename(&tmp, &path).context("renaming static")?;
    Ok(())
}

pub(super) fn cache_jar_path(storage: &Path, sha1: &str) -> Result<PathBuf> {
    if sha1.len() != 40 || !sha1.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid sha1: {sha1}");
    }
    let prefix = &sha1[..2];
    Ok(storage
        .join("cache")
        .join(prefix)
        .join(format!("{sha1}.jar")))
}

pub(super) fn static_asset_path(storage: &Path, pack_id: &str, rel_path: &str) -> Result<PathBuf> {
    if rel_path.contains("..") || rel_path.starts_with('/') {
        bail!("invalid static rel_path: {rel_path}");
    }
    Ok(storage
        .join("packs")
        .join(pack_id)
        .join("static")
        .join(rel_path))
}

pub(super) fn cache_url(base: &str, sha1: &str) -> String {
    // sha1 is hex-only by construction (verified upstream); no encoding
    // needed for path segments here.
    let prefix = &sha1[..2];
    let base = base.trim_end_matches('/');
    format!("{base}/v1/cache/{prefix}/{sha1}.jar")
}

pub(super) fn static_url(base: &str, pack_id: &str, rel_path: &str) -> String {
    // rel_path may contain spaces, parens, plus, comma (storage's
    // validate_rel_path allows them since real resourcepack and
    // shaderpack filenames carry such characters). Manifest URLs are
    // consumed by strict HTTP clients (Java's URI, kotlinx ktor, Rust
    // reqwest) that reject raw spaces with HTTP 400 from nginx or
    // outright parse failures. Percent-encode every segment so the
    // published URL is RFC 3986-compliant; segment boundaries (/)
    // stay unencoded so the path structure survives.
    let base = base.trim_end_matches('/');
    let pack_enc = url_encode_segment(pack_id);
    let rel_enc = rel_path
        .split('/')
        .map(url_encode_segment)
        .collect::<Vec<_>>()
        .join("/");
    format!("{base}/v1/packs/{pack_enc}/static/{rel_enc}")
}

/// Percent-encode a single path segment using the RFC 3986 unreserved
/// set plus sub-delims, minus the path-structural ones. Equivalent in
/// scope to JavaScript's `encodeURIComponent` -- safe to drop into any
/// URL position that holds a single segment.
fn url_encode_segment(s: &str) -> String {
    use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
    // RFC 3986: pchar = unreserved / pct-encoded / sub-delims / ":" / "@"
    // We additionally encode "/", "?", "#", "[", "]", "&", "=" (would
    // change URL meaning), space (must always encode), and "%" (would
    // collide with already-encoded sequences).
    const SET: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'<')
        .add(b'>')
        .add(b'?')
        .add(b'[')
        .add(b'\\')
        .add(b']')
        .add(b'^')
        .add(b'`')
        .add(b'{')
        .add(b'|')
        .add(b'}')
        .add(b'/')
        .add(b'&')
        .add(b'=')
        .add(b'+');
    utf8_percent_encode(s, SET).to_string()
}

pub(super) fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_url_percent_encodes_spaces_and_special_chars() {
        let url = static_url(
            "https://smrt.hivens.dev",
            "Industrial",
            "shaderpacks/Chocapic13 V7.1 High.zip",
        );
        assert_eq!(
            url,
            "https://smrt.hivens.dev/v1/packs/Industrial/static/shaderpacks/Chocapic13%20V7.1%20High.zip"
        );
    }

    #[test]
    fn static_url_keeps_segment_boundaries_unencoded() {
        // The "/" between segments stays as path separator, only the
        // segments themselves get encoded. Catches a regression where
        // someone naively percent-encodes the whole rel_path including
        // its slashes.
        let url = static_url("https://m.example", "pack", "a/b c/d.txt");
        assert_eq!(url, "https://m.example/v1/packs/pack/static/a/b%20c/d.txt");
    }

    #[test]
    fn static_url_encodes_parens_and_plus() {
        let url = static_url("https://m.example", "p", "shaderpacks/BSL (v8+).zip");
        // parens stay literal in this set (allowed by RFC 3986 sub-delims
        // and ktor/reqwest parse them fine); plus encodes to %2B because
        // it has special meaning in query strings and some parsers
        // mistranslate it to space.
        assert!(url.contains("BSL%20(v8%2B).zip"), "got {url}");
    }
}
