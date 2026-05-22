use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use smrt::types::*;
use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use tracing::{info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const MODRINTH_BATCH_SIZE: usize = 100;
const MODRINTH_API: &str = "https://api.modrinth.com/v2/version_files";
const USER_AGENT: &str = "Kitty-Hivens/smrt-ingest (https://github.com/Kitty-Hivens/smrt)";

#[derive(Parser, Debug)]
#[command(about = "Ingest a SmartyCraft pack archive into the smrt mirror's storage layout.")]
struct Args {
    /// Local path or HTTP(S) URL to the SmartyCraft pack archive.
    #[arg(long)]
    sc_archive: String,

    /// Path to a JSON file with per-pack metadata. See PackConfig in this file.
    #[arg(long)]
    config: PathBuf,

    /// Storage root, matches SMRT_STORAGE_DIR of the running smrt service.
    #[arg(long, default_value = "/var/lib/smrt")]
    storage: PathBuf,

    /// Pack version slug. Defaults to today's UTC date as YYYY.MM.DD.
    #[arg(long)]
    pack_version: Option<String>,

    /// Base URL the mirror serves on. Used to construct smrt_cache source URLs.
    #[arg(long, default_value = "https://smrt.hivens.dev")]
    mirror_base_url: String,
}

#[derive(Deserialize, Debug)]
struct PackConfig {
    pack_id: String,
    display_name: String,
    tagline: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    featured: bool,
    minecraft_version: String,
    loader: LoaderSpec,
    java_major: u32,
}

struct DiscoveredMod {
    sha1: String,
    filename: String,
    bytes: Vec<u8>,
}

#[derive(Deserialize)]
struct ModrinthVersionFile {
    project_id: String,
    id: String, // version_id
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "smrt_ingest=info,info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();
    run(args).await
}

async fn run(args: Args) -> Result<()> {
    let pack_config: PackConfig = {
        let bytes = fs::read(&args.config)
            .with_context(|| format!("reading {}", args.config.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing {}", args.config.display()))?
    };
    info!(pack_id = %pack_config.pack_id, "loaded pack config");

    let archive_bytes = obtain_archive(&args.sc_archive).await?;
    info!(bytes = archive_bytes.len(), "loaded SC archive");

    let mods = extract_mods(&archive_bytes)?;
    info!(count = mods.len(), "discovered mods in archive");
    if mods.is_empty() {
        return Err(anyhow!("no mods/*.jar found in archive -- check archive layout"));
    }

    let modrinth_hits = modrinth_batch_lookup(&mods).await?;
    let modrinth_count = modrinth_hits.len();
    let cache_count = mods.len() - modrinth_count;
    info!(modrinth = modrinth_count, smrt_cache = cache_count, "classified mods");

    let pack_version = args
        .pack_version
        .clone()
        .unwrap_or_else(today_slug);

    // ── filesystem layout ──────────────────────────────────────────────────
    let pack_dir = args.storage.join("packs").join(&pack_config.pack_id);
    let manifests_dir = pack_dir.join("manifests");
    let extras_dir = pack_dir.join("extras");
    let cache_dir = args.storage.join("cache");
    fs::create_dir_all(&manifests_dir).context("creating manifests dir")?;
    fs::create_dir_all(&extras_dir).context("creating extras dir")?;
    fs::create_dir_all(&cache_dir).context("creating cache dir")?;

    // ── mod entries: modrinth source where matched, else copy into smrt_cache
    let mut mod_entries: Vec<ModEntry> = Vec::with_capacity(mods.len());
    for m in &mods {
        let source = if let Some(hit) = modrinth_hits.get(&m.sha1) {
            ModSource::Modrinth {
                project_id: hit.project_id.clone(),
                version_id: hit.id.clone(),
            }
        } else {
            write_to_cache(&cache_dir, &m.sha1, &m.bytes)?;
            ModSource::SmrtCache {
                url: cache_url(&args.mirror_base_url, &m.sha1),
            }
        };
        mod_entries.push(ModEntry {
            sha1: m.sha1.clone(),
            filename: m.filename.clone(),
            size_bytes: m.bytes.len() as u64,
            required: true,
            sources: vec![source],
        });
    }
    // Stable order so manifest diffs across versions stay readable.
    mod_entries.sort_by(|a, b| a.filename.cmp(&b.filename));

    // ── extras: copy SC's `extra.zip` if present
    let extras_ref = if let Some(extra_zip_bytes) = extract_extra_zip(&archive_bytes)? {
        let extras_path = extras_dir.join(format!("{}.zip", pack_version));
        fs::write(&extras_path, &extra_zip_bytes).context("writing extras zip")?;
        let extras_sha1 = sha1_hex(&extra_zip_bytes);
        Some(ExtrasRef {
            url: format!(
                "{}/v1/packs/{}/extras/{}.zip",
                args.mirror_base_url, pack_config.pack_id, pack_version
            ),
            sha1: extras_sha1,
            size_bytes: extra_zip_bytes.len() as u64,
        })
    } else {
        None
    };

    // ── manifest
    let manifest = PackManifest {
        schema_version: SCHEMA_VERSION,
        pack_id: pack_config.pack_id.clone(),
        pack_version: pack_version.clone(),
        generated_at: now_rfc3339(),
        minecraft: MinecraftSpec {
            version: pack_config.minecraft_version.clone(),
        },
        loader: pack_config.loader.clone(),
        java: JavaSpec {
            major: pack_config.java_major,
        },
        mods: mod_entries,
        extras: extras_ref,
    };

    let manifest_filename = format!("{}.json", pack_version);
    let manifest_path = manifests_dir.join(&manifest_filename);
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
        .context("writing manifest")?;

    // ── update `latest` symlink atomically (write to .tmp + rename)
    let latest_path = manifests_dir.join("latest");
    let latest_tmp = manifests_dir.join("latest.tmp");
    let _ = fs::remove_file(&latest_tmp);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&manifest_filename, &latest_tmp)
        .context("creating latest.tmp symlink")?;
    fs::rename(&latest_tmp, &latest_path).context("swapping latest symlink")?;

    // ── summary
    let summary = PackSummary {
        pack_id: pack_config.pack_id.clone(),
        display_name: pack_config.display_name.clone(),
        tagline: pack_config.tagline.clone(),
        minecraft_version: pack_config.minecraft_version.clone(),
        latest_pack_version: pack_version.clone(),
        tags: pack_config.tags.clone(),
        featured: pack_config.featured,
    };
    fs::write(
        pack_dir.join("summary.json"),
        serde_json::to_string_pretty(&summary)?,
    )
    .context("writing summary")?;

    info!(
        pack_id = %pack_config.pack_id,
        pack_version = %pack_version,
        mods_total = mods.len(),
        modrinth_sourced = modrinth_count,
        smrt_cached = cache_count,
        extras_present = manifest.extras.is_some(),
        "ingest complete"
    );
    Ok(())
}

// ── archive acquisition ───────────────────────────────────────────────────

async fn obtain_archive(source: &str) -> Result<Vec<u8>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        info!(url = source, "downloading SC archive");
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()?;
        let resp = client.get(source).send().await?.error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    } else {
        info!(path = source, "reading SC archive from disk");
        Ok(fs::read(source).with_context(|| format!("reading {source}"))?)
    }
}

// ── mod discovery ─────────────────────────────────────────────────────────

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
        // mods/X.jar OR mods/subdir/X.jar (some SC packs nest by MC version).
        let segments: Vec<&str> = name.split('/').collect();
        let is_mod_jar = segments.first() == Some(&"mods")
            && name.ends_with(".jar")
            && segments.last().map(|s| !s.is_empty()).unwrap_or(false);
        if !is_mod_jar {
            continue;
        }
        let filename = segments.last().unwrap().to_string();
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes)
            .with_context(|| format!("reading mod jar {name}"))?;
        let sha1 = sha1_hex(&bytes);
        out.push(DiscoveredMod { sha1, filename, bytes });
    }
    Ok(out)
}

fn extract_extra_zip(archive_bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let reader = Cursor::new(archive_bytes);
    let mut zip = zip::ZipArchive::new(reader).context("opening SC archive as zip")?;
    let mut entry = match zip.by_name("extra.zip") {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).context("reading extra.zip")?;
    Ok(Some(bytes))
}

// ── modrinth lookup ───────────────────────────────────────────────────────

async fn modrinth_batch_lookup(
    mods: &[DiscoveredMod],
) -> Result<HashMap<String, ModrinthVersionFile>> {
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;
    let mut out: HashMap<String, ModrinthVersionFile> = HashMap::new();
    let sha1s: Vec<&str> = mods.iter().map(|m| m.sha1.as_str()).collect();

    for chunk in sha1s.chunks(MODRINTH_BATCH_SIZE) {
        let body = serde_json::json!({
            "hashes": chunk,
            "algorithm": "sha1",
        });
        let resp = client.post(MODRINTH_API).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(%status, body = %text, "modrinth batch lookup failed");
            return Err(anyhow!("modrinth lookup HTTP {status}"));
        }
        let map: HashMap<String, ModrinthVersionFile> = resp.json().await?;
        out.extend(map);
    }
    Ok(out)
}

// ── helpers ───────────────────────────────────────────────────────────────

fn write_to_cache(cache_dir: &std::path::Path, sha1: &str, bytes: &[u8]) -> Result<()> {
    let prefix = &sha1[..2];
    let dir = cache_dir.join(prefix);
    fs::create_dir_all(&dir).context("creating cache prefix dir")?;
    let path = dir.join(format!("{sha1}.jar"));
    // Skip rewrite if content already there (idempotent re-ingests).
    if path.exists() {
        return Ok(());
    }
    let tmp = path.with_extension("jar.tmp");
    fs::write(&tmp, bytes).context("writing cache jar tmp")?;
    fs::rename(&tmp, &path).context("renaming cache jar into place")?;
    Ok(())
}

fn cache_url(base: &str, sha1: &str) -> String {
    let prefix = &sha1[..2];
    let base = base.trim_end_matches('/');
    format!("{base}/v1/cache/{prefix}/{sha1}.jar")
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
