use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use sha1::{Digest, Sha1};
use smrt::modrinth::{Modrinth, Version as MrVersion};
use smrt::pack_config::{DeclaredAsset, DeclaredMod, PackConfig, SourceDecl};
use smrt::types::{
    AssetEntry, JavaSpec, LoaderSpec, MinecraftSpec, ModEntry, PackManifest, PackSummary, Source,
    SCHEMA_VERSION,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const DEFAULT_MIRROR_BASE: &str = "https://smrt.hivens.dev";

#[derive(Parser, Debug)]
#[command(name = "smrt-pack", version, about = "Authoring CLI for smrt packs")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Read an SC archive and emit a starter PackConfig JSON.
    Bootstrap {
        #[arg(long)]
        sc_archive: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        pack_id: String,
        #[arg(long)]
        display_name: String,
        #[arg(long, default_value = "")]
        tagline: String,
        #[arg(long)]
        minecraft_version: String,
        #[arg(long, default_value = "forge")]
        loader_name: String,
        #[arg(long)]
        loader_version: String,
        #[arg(long, default_value_t = 8)]
        java_major: u32,
        /// Storage root: extracted mod jars land in {storage}/cache/, extras
        /// files land in {storage}/packs/{pack_id}/static/.
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
    },

    /// Cross-reference a PackConfig against an SC archive by filename.
    Validate {
        #[arg(long)]
        config: PathBuf,
        #[arg(long = "against-sc-archive")]
        sc_archive: PathBuf,
    },

    /// Resolve every source in a PackConfig and write the wire manifest.
    Build {
        #[arg(long)]
        config: PathBuf,
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
        /// Defaults to today's UTC date in YYYY.MM.DD form.
        #[arg(long)]
        pack_version: Option<String>,
        #[arg(long, default_value = DEFAULT_MIRROR_BASE)]
        mirror_base: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "smrt_pack=info,info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Bootstrap {
            sc_archive,
            out,
            pack_id,
            display_name,
            tagline,
            minecraft_version,
            loader_name,
            loader_version,
            java_major,
            storage,
        } => {
            bootstrap(BootstrapArgs {
                sc_archive,
                out,
                pack_id,
                display_name,
                tagline,
                minecraft_version,
                loader: LoaderSpec {
                    name: loader_name,
                    version: loader_version,
                },
                java_major,
                storage,
            })
            .await
        }
        Cmd::Validate { config, sc_archive } => validate(&config, &sc_archive),
        Cmd::Build {
            config,
            storage,
            pack_version,
            mirror_base,
        } => build(&config, &storage, pack_version.as_deref(), &mirror_base).await,
    }
}

// ── bootstrap ──────────────────────────────────────────────────────────────

struct BootstrapArgs {
    sc_archive: PathBuf,
    out: PathBuf,
    pack_id: String,
    display_name: String,
    tagline: String,
    minecraft_version: String,
    loader: LoaderSpec,
    java_major: u32,
    storage: PathBuf,
}

struct DiscoveredMod {
    sha1: String,
    filename: String,
    bytes: Vec<u8>,
}

struct DiscoveredAsset {
    rel_path: String,
    bytes: Vec<u8>,
}

async fn bootstrap(args: BootstrapArgs) -> Result<()> {
    let archive_bytes = fs::read(&args.sc_archive)
        .with_context(|| format!("reading {}", args.sc_archive.display()))?;
    info!(bytes = archive_bytes.len(), "loaded SC archive");

    let mods = extract_mods(&archive_bytes)?;
    info!(count = mods.len(), "discovered mods in archive");
    if mods.is_empty() {
        bail!("no mods/*.jar in archive -- wrong archive layout?");
    }

    let extras = extract_extra_assets(&archive_bytes)?;
    info!(count = extras.len(), "discovered extras files");

    let modrinth = Modrinth::new()?;
    let sha1s: Vec<String> = mods.iter().map(|m| m.sha1.clone()).collect();
    let hits = modrinth.version_files_by_sha1(&sha1s).await?;
    info!(matched = hits.len(), total = sha1s.len(), "modrinth lookup");

    let cache_dir = args.storage.join("cache");
    let static_dir = args.storage.join("packs").join(&args.pack_id).join("static");

    let mut declared_mods = Vec::with_capacity(mods.len());
    for m in &mods {
        let decl = if let Some(hit) = hits.get(&m.sha1) {
            let mc_ok = hit.game_versions.iter().any(|v| v == &args.minecraft_version);
            let loader_ok = hit.loaders.iter().any(|l| l.eq_ignore_ascii_case(&args.loader.name));
            if mc_ok && loader_ok {
                DeclaredMod {
                    filename: m.filename.clone(),
                    required: true,
                    source: SourceDecl::Modrinth {
                        project_id: hit.project_id.clone(),
                        version_id: hit.id.clone(),
                    },
                    note: Some(format!("matched on Modrinth ({})", hit.version_number)),
                }
            } else {
                write_to_cache(&cache_dir, &m.sha1, &m.bytes)?;
                DeclaredMod {
                    filename: m.filename.clone(),
                    required: true,
                    source: SourceDecl::SmrtCache { sha1: m.sha1.clone() },
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
                source: SourceDecl::SmrtCache { sha1: m.sha1.clone() },
                note: Some("TODO: no Modrinth match; check if a relabel of an upstream project".into()),
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
            source: SourceDecl::SmrtStatic { rel_path: a.rel_path.clone() },
            note: Some("TODO: review whether to keep SC default or curate replacement".into()),
        });
    }
    declared_assets.sort_by(|a, b| a.dest.cmp(&b.dest));

    let cfg = PackConfig {
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
    };

    let pretty = serde_json::to_string_pretty(&cfg)?;
    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&args.out, pretty)
        .with_context(|| format!("writing {}", args.out.display()))?;
    info!(
        path = %args.out.display(),
        mods = cfg.mods.len(),
        assets = cfg.assets.len(),
        "wrote starter config"
    );
    Ok(())
}

// ── validate ───────────────────────────────────────────────────────────────

fn validate(config_path: &Path, sc_archive_path: &Path) -> Result<()> {
    let cfg: PackConfig = read_json(config_path)?;
    let archive_bytes = fs::read(sc_archive_path)
        .with_context(|| format!("reading {}", sc_archive_path.display()))?;
    let sc_mods = extract_mods(&archive_bytes)?;

    let sc_filenames: HashSet<&str> = sc_mods.iter().map(|m| m.filename.as_str()).collect();
    let config_filenames: HashSet<&str> = cfg.mods.iter().map(|m| m.filename.as_str()).collect();

    let missing_in_config: Vec<&&str> = sc_filenames.difference(&config_filenames).collect();
    let extra_in_config: Vec<&&str> = config_filenames.difference(&sc_filenames).collect();
    let matched = sc_filenames.intersection(&config_filenames).count();

    println!("SC archive: {} mods", sc_mods.len());
    println!("PackConfig: {} mods declared, {} assets declared", cfg.mods.len(), cfg.assets.len());
    println!("matched by filename: {}", matched);

    if !missing_in_config.is_empty() {
        println!("\nIn SC archive but missing from PackConfig (would break handshake):");
        let mut sorted: Vec<&&&str> = missing_in_config.iter().collect();
        sorted.sort();
        for f in sorted {
            println!("  - {}", f);
        }
    }
    if !extra_in_config.is_empty() {
        println!("\nIn PackConfig but not in SC archive (client additions, expected if intentional):");
        let mut sorted: Vec<&&&str> = extra_in_config.iter().collect();
        sorted.sort();
        for f in sorted {
            println!("  + {}", f);
        }
    }

    if !missing_in_config.is_empty() {
        bail!("{} SC mods missing from PackConfig", missing_in_config.len());
    }
    Ok(())
}

// ── build ──────────────────────────────────────────────────────────────────

async fn build(
    config_path: &Path,
    storage: &Path,
    pack_version: Option<&str>,
    mirror_base: &str,
) -> Result<()> {
    let cfg: PackConfig = read_json(config_path)?;
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
        asset_entries.push(resolve_asset(a, &cfg.pack_id, storage, mirror_base, &modrinth, &modrinth_cache).await?);
    }
    asset_entries.sort_by(|a, b| a.dest.cmp(&b.dest));

    let manifest = PackManifest {
        schema_version: SCHEMA_VERSION,
        pack_id: cfg.pack_id.clone(),
        pack_version: pack_version.clone(),
        generated_at: now_rfc3339(),
        minecraft: MinecraftSpec { version: cfg.minecraft_version.clone() },
        loader: cfg.loader.clone(),
        java: JavaSpec { major: cfg.java_major },
        mods: mod_entries,
        assets: asset_entries,
    };

    write_manifest(&manifest, storage, &cfg, &pack_version)?;
    info!(pack_version = %pack_version, "build complete");
    Ok(())
}

#[derive(Default)]
struct ModrinthCache {
    inner: tokio::sync::Mutex<HashMap<(String, String), MrVersion>>,
}

impl ModrinthCache {
    async fn get_or_fetch(
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

async fn resolve_mod(
    decl: &DeclaredMod,
    storage: &Path,
    mirror_base: &str,
    modrinth: &Modrinth,
    cache: &ModrinthCache,
) -> Result<ModEntry> {
    let (sha1, size_bytes, source) = match &decl.source {
        SourceDecl::Modrinth { project_id, version_id } => {
            let v = cache.get_or_fetch(modrinth, project_id, version_id).await
                .with_context(|| format!("resolving Modrinth mod {}", decl.filename))?;
            let f = v
                .primary_file()
                .ok_or_else(|| anyhow!("Modrinth version {project_id}/{version_id} has no files"))?;
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
            let meta = fs::metadata(&path)
                .with_context(|| format!("cache jar {} not found for mod {}", path.display(), decl.filename))?;
            (sha1.clone(), meta.len(), Source::SmrtCache { url: cache_url(mirror_base, sha1) })
        }
        SourceDecl::SmrtStatic { .. } => {
            bail!("mod {} uses smrt_static source -- mods must be modrinth or smrt_cache", decl.filename);
        }
    };

    Ok(ModEntry {
        filename: decl.filename.clone(),
        sha1,
        size_bytes,
        required: decl.required,
        source,
    })
}

async fn resolve_asset(
    decl: &DeclaredAsset,
    pack_id: &str,
    storage: &Path,
    mirror_base: &str,
    modrinth: &Modrinth,
    cache: &ModrinthCache,
) -> Result<AssetEntry> {
    let (sha1, size_bytes, source) = match &decl.source {
        SourceDecl::Modrinth { project_id, version_id } => {
            let v = cache.get_or_fetch(modrinth, project_id, version_id).await
                .with_context(|| format!("resolving Modrinth asset {}", decl.dest))?;
            let f = v
                .primary_file()
                .ok_or_else(|| anyhow!("Modrinth version {project_id}/{version_id} has no files"))?;
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
            let bytes = fs::read(&path)
                .with_context(|| format!("static asset {} not found for {}", path.display(), decl.dest))?;
            let size = bytes.len() as u64;
            let sha = sha1_hex(&bytes);
            (sha, size, Source::SmrtStatic { url: static_url(mirror_base, pack_id, rel_path) })
        }
        SourceDecl::SmrtCache { .. } => {
            bail!("asset {} uses smrt_cache source -- assets must be modrinth or smrt_static", decl.dest);
        }
    };

    Ok(AssetEntry {
        dest: decl.dest.clone(),
        sha1,
        size_bytes,
        required: decl.required,
        source,
    })
}

fn write_manifest(manifest: &PackManifest, storage: &Path, cfg: &PackConfig, pack_version: &str) -> Result<()> {
    let pack_dir = storage.join("packs").join(&cfg.pack_id);
    let manifests_dir = pack_dir.join("manifests");
    fs::create_dir_all(&manifests_dir).context("creating manifests dir")?;

    let filename = format!("{pack_version}.json");
    let manifest_path = manifests_dir.join(&filename);
    fs::write(&manifest_path, serde_json::to_string_pretty(manifest)?)
        .context("writing manifest")?;

    // Atomic symlink swap so concurrent readers never see a missing `latest`.
    let latest_path = manifests_dir.join("latest");
    let latest_tmp = manifests_dir.join("latest.tmp");
    let _ = fs::remove_file(&latest_tmp);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&filename, &latest_tmp).context("symlinking latest.tmp")?;
    fs::rename(&latest_tmp, &latest_path).context("renaming latest")?;

    let summary = PackSummary {
        pack_id: cfg.pack_id.clone(),
        display_name: cfg.display_name.clone(),
        tagline: cfg.tagline.clone(),
        minecraft_version: cfg.minecraft_version.clone(),
        latest_pack_version: pack_version.to_string(),
        tags: cfg.tags.clone(),
        featured: cfg.featured,
    };
    fs::write(pack_dir.join("summary.json"), serde_json::to_string_pretty(&summary)?)
        .context("writing summary")?;
    Ok(())
}

// ── archive helpers ────────────────────────────────────────────────────────

fn extract_mods(archive_bytes: &[u8]) -> Result<Vec<DiscoveredMod>> {
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
        entry.read_to_end(&mut bytes).with_context(|| format!("reading {name}"))?;
        let sha1 = sha1_hex(&bytes);
        out.push(DiscoveredMod { sha1, filename, bytes });
    }
    Ok(out)
}

fn extract_extra_assets(archive_bytes: &[u8]) -> Result<Vec<DiscoveredAsset>> {
    let reader = Cursor::new(archive_bytes);
    let mut zip = zip::ZipArchive::new(reader).context("opening SC archive as zip")?;
    let mut extra_zip_bytes = None;
    if let Ok(mut entry) = zip.by_name("extra.zip") {
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf).context("reading extra.zip")?;
        extra_zip_bytes = Some(buf);
    }
    let Some(bytes) = extra_zip_bytes else { return Ok(Vec::new()); };

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
        entry.read_to_end(&mut buf).with_context(|| format!("reading extra entry {name}"))?;
        out.push(DiscoveredAsset { rel_path: name, bytes: buf });
    }
    Ok(out)
}

// ── filesystem helpers ─────────────────────────────────────────────────────

fn write_to_cache(cache_dir: &Path, sha1: &str, bytes: &[u8]) -> Result<()> {
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

fn write_to_static(static_dir: &Path, rel_path: &str, bytes: &[u8]) -> Result<()> {
    let path = static_dir.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating static parent dir")?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).context("writing static tmp")?;
    fs::rename(&tmp, &path).context("renaming static")?;
    Ok(())
}

fn cache_jar_path(storage: &Path, sha1: &str) -> Result<PathBuf> {
    if sha1.len() != 40 || !sha1.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid sha1: {sha1}");
    }
    let prefix = &sha1[..2];
    Ok(storage.join("cache").join(prefix).join(format!("{sha1}.jar")))
}

fn static_asset_path(storage: &Path, pack_id: &str, rel_path: &str) -> Result<PathBuf> {
    if rel_path.contains("..") || rel_path.starts_with('/') {
        bail!("invalid static rel_path: {rel_path}");
    }
    Ok(storage.join("packs").join(pack_id).join("static").join(rel_path))
}

fn cache_url(base: &str, sha1: &str) -> String {
    let prefix = &sha1[..2];
    let base = base.trim_end_matches('/');
    format!("{base}/v1/cache/{prefix}/{sha1}.jar")
}

fn static_url(base: &str, pack_id: &str, rel_path: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/v1/packs/{pack_id}/static/{rel_path}")
}

// ── misc ───────────────────────────────────────────────────────────────────

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn now_rfc3339() -> String {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;
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

fn today_slug() -> String {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_utc();
    format!("{:04}.{:02}.{:02}", now.year(), u8::from(now.month()), now.day())
}
