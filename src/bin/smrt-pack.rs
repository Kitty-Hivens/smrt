use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use smrt::authoring::{
    self, BootstrapArgs, Modrinth, apply_curator as enrich_apply_curator,
    apply_role_table as enrich_apply_role_table, enrich_from_mcmod_info,
    infer_requires_from_mcmod_info, load_curator, load_pack_meta, load_role_table,
};
use smrt::domain::{LoaderSpec, PackConfig, PackManifest, PackSummary};
use smrt::registry::Registry;
use smrt::storage::Storage;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
        /// Optional TOML with icon_url / banner_url / gallery_urls /
        /// description_md fields. Merged into summary.json when present.
        #[arg(long)]
        pack_meta: Option<PathBuf>,
    },

    /// Fill display.name / description / url from each smrt_cache mod's
    /// `mcmod.info`. Existing curator-written values win. Idempotent.
    EnrichMcmod {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
    },

    /// Apply a TOML role table (filename -> role) to `display.role`
    /// across the pack. Existing values win; unmatched table entries
    /// are reported so the curator can spot typos.
    ApplyRoleTable {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        table: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },

    /// Walk each mod's `mcmod.info.dependencies`, resolve modids
    /// against sibling mods in the pack, emit `display.requires`
    /// entries. Existing `requires` lists are preserved.
    InferRequires {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
    },

    /// Run the full curator chain (enrich-mcmod -> role-table ->
    /// category-table -> mark-optional -> substitute -> infer-requires
    /// -> extras). Reads a single curator.toml that absorbs every
    /// per-pack curator decision.
    ApplyCurator {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        curator: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
        /// MC version used to filter Modrinth lookups for extras. Falls
        /// back to the PackConfig's `minecraft_version` when omitted.
        #[arg(long)]
        mc_version: Option<String>,
    },

    /// PUT a local directory tree into the mirror's pack static area
    /// via the admin API. Walks every regular file under [dir] and
    /// publishes each at `/v1/admin/packs/<pack_id>/static/<rel_path>`
    /// (relative to dir). Reads the admin token from [token_file]
    /// (default `/tmp/smrt-token`).
    UploadStatic {
        #[arg(long)]
        pack_id: String,
        #[arg(long)]
        dir: PathBuf,
        #[arg(long, default_value = DEFAULT_MIRROR_BASE)]
        mirror_base: String,
        #[arg(long, default_value = "/tmp/smrt-token")]
        token_file: PathBuf,
        /// Skip files matching any of these path prefixes (relative
        /// to [dir]). Repeatable. Default skips obvious junk
        /// (`.DS_Store`, `Thumbs.db`).
        #[arg(long = "skip", default_values = [".DS_Store", "Thumbs.db"])]
        skip: Vec<String>,
    },

    /// Reconstruct an editable authoring config from a published manifest +
    /// summary, to migrate a CLI-era pack (no `authoring/` inputs) into the
    /// panel's editable format. Pair with a curator that folds in pack-meta.
    ReconstructConfig {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        summary: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },

    /// Mod-identity registry: harvest the cache + manifests into SQLite, or
    /// inspect it.
    Registry {
        #[command(subcommand)]
        sub: RegistryCmd,
    },
}

#[derive(Subcommand, Debug)]
enum RegistryCmd {
    /// Scan the cache + published manifests, read mcmod.info, resolve Modrinth
    /// identity, and reconcile into the registry DB. Idempotent; never
    /// clobbers authored rows.
    Harvest {
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
    },
    /// Print registry counts (mods / versions / relations / packs / builds / orphans).
    Stats {
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
    },
    /// List cached artifacts no build references.
    Orphans {
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
    },
    /// Mark a pack's provenance (sc | hivens) as an operator decision.
    Provenance {
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
        #[arg(long)]
        pack: String,
        #[arg(long = "as", value_parser = ["sc", "hivens"])]
        provenance: String,
    },
    /// Add (or --remove) a mutual authored conflict between two mods, by modid.
    Conflict {
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
        #[arg(long)]
        a: String,
        #[arg(long)]
        b: String,
        #[arg(long)]
        remove: bool,
    },
    /// Snapshot the registry DB to a file (VACUUM INTO).
    Backup {
        #[arg(long, default_value = "/var/lib/smrt")]
        storage: PathBuf,
        #[arg(long)]
        out: PathBuf,
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
            let archive = fs::read(&sc_archive)
                .with_context(|| format!("reading {}", sc_archive.display()))?;
            let cfg = authoring::bootstrap(
                BootstrapArgs {
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
                },
                archive,
            )
            .await?;
            write_pack_config(&cfg, &out)?;
            info!(
                path = %out.display(),
                mods = cfg.mods.len(),
                assets = cfg.assets.len(),
                "wrote starter config"
            );
            Ok(())
        }
        Cmd::Validate { config, sc_archive } => run_validate(&config, &sc_archive),
        Cmd::Build {
            config,
            storage,
            pack_version,
            mirror_base,
            pack_meta,
        } => {
            run_build(
                &config,
                &storage,
                pack_version.as_deref(),
                &mirror_base,
                pack_meta.as_deref(),
            )
            .await
        }
        Cmd::EnrichMcmod {
            config,
            out,
            storage,
        } => run_enrich_mcmod(&config, &out, &storage),
        Cmd::ApplyRoleTable { config, table, out } => run_apply_role_table(&config, &table, &out),
        Cmd::InferRequires {
            config,
            out,
            storage,
        } => run_infer_requires(&config, &out, &storage),
        Cmd::ApplyCurator {
            config,
            curator,
            out,
            storage,
            mc_version,
        } => run_apply_curator(&config, &curator, &out, &storage, mc_version.as_deref()).await,
        Cmd::UploadStatic {
            pack_id,
            dir,
            mirror_base,
            token_file,
            skip,
        } => run_upload_static(&pack_id, &dir, &mirror_base, &token_file, &skip).await,
        Cmd::ReconstructConfig {
            manifest,
            summary,
            out,
        } => run_reconstruct_config(&manifest, &summary, &out),
        Cmd::Registry { sub } => match sub {
            RegistryCmd::Harvest { storage } => run_registry_harvest(&storage).await,
            RegistryCmd::Stats { storage } => run_registry_stats(&storage),
            RegistryCmd::Orphans { storage } => run_registry_orphans(&storage),
            RegistryCmd::Provenance {
                storage,
                pack,
                provenance,
            } => run_registry_provenance(&storage, &pack, &provenance),
            RegistryCmd::Conflict {
                storage,
                a,
                b,
                remove,
            } => run_registry_conflict(&storage, &a, &b, remove),
            RegistryCmd::Backup { storage, out } => run_registry_backup(&storage, &out),
        },
    }
}

// ── registry ────────────────────────────────────────────────────────────────

async fn run_registry_harvest(storage: &Path) -> Result<()> {
    let store = Storage::new(storage.to_path_buf());
    let modrinth = Modrinth::new()?;
    let registry = Arc::new(Registry::open(storage.join("registry.db"))?);
    let report = authoring::harvest::run_harvest(&store, &modrinth, registry).await?;
    info!(
        jars = report.jars_scanned,
        no_identity = report.jars_no_identity,
        mods = report.mods,
        versions = report.mod_versions,
        relations = report.relations,
        builds = report.builds,
        "harvest complete"
    );
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn run_registry_stats(storage: &Path) -> Result<()> {
    let registry = Registry::open(storage.join("registry.db"))?;
    let stats = registry.with_conn(smrt::registry::queries::stats)?;
    println!("{}", serde_json::to_string_pretty(&stats)?);
    Ok(())
}

fn run_registry_orphans(storage: &Path) -> Result<()> {
    let registry = Registry::open(storage.join("registry.db"))?;
    let orphans = registry.with_conn(smrt::registry::queries::orphan_jars)?;
    for o in &orphans {
        println!(
            "{}  {:>11} B  {}",
            o.sha1,
            o.size_bytes,
            o.filename.as_deref().unwrap_or("(no name)")
        );
    }
    println!("{} orphan(s)", orphans.len());
    Ok(())
}

fn run_registry_provenance(storage: &Path, pack: &str, provenance: &str) -> Result<()> {
    let registry = Registry::open(storage.join("registry.db"))?;
    registry.set_provenance(pack, provenance)?;
    info!(pack, provenance, "set pack provenance");
    Ok(())
}

fn run_registry_conflict(storage: &Path, a: &str, b: &str, remove: bool) -> Result<()> {
    let registry = Registry::open(storage.join("registry.db"))?;
    registry.set_conflict(a, b, remove)?;
    info!(a, b, remove, "set authored conflict");
    Ok(())
}

fn run_registry_backup(storage: &Path, out: &Path) -> Result<()> {
    let registry = Registry::open(storage.join("registry.db"))?;
    registry.backup_into(out)?;
    info!(out = %out.display(), "registry backup written");
    Ok(())
}

fn run_reconstruct_config(manifest_path: &Path, summary_path: &Path, out: &Path) -> Result<()> {
    let manifest: PackManifest = read_json(manifest_path)?;
    let summary: PackSummary = read_json(summary_path)?;
    let cfg = authoring::reconstruct_config(&manifest, &summary);
    let json = serde_json::to_vec_pretty(&cfg).context("encoding reconstructed config")?;
    fs::write(out, &json).with_context(|| format!("writing {}", out.display()))?;
    info!(
        out = %out.display(),
        mods = cfg.mods.len(),
        assets = cfg.assets.len(),
        "reconstructed editable config from manifest + summary"
    );
    Ok(())
}

// ── build + validate (thin wrappers over authoring::) ──────────────────────

fn run_validate(config_path: &Path, sc_archive_path: &Path) -> Result<()> {
    let cfg: PackConfig = read_json(config_path)?;
    let archive_bytes = fs::read(sc_archive_path)
        .with_context(|| format!("reading {}", sc_archive_path.display()))?;
    let report = authoring::validate(&cfg, &archive_bytes)?;

    println!("SC archive: {} mods", report.sc_mod_count);
    println!(
        "PackConfig: {} mods declared, {} assets declared",
        report.declared_mods, report.declared_assets
    );
    println!("matched by filename: {}", report.matched);

    if !report.missing_in_config.is_empty() {
        println!("\nIn SC archive but missing from PackConfig (would break handshake):");
        for f in &report.missing_in_config {
            println!("  - {}", f);
        }
    }
    if !report.extra_in_config.is_empty() {
        println!(
            "\nIn PackConfig but not in SC archive (client additions, expected if intentional):"
        );
        for f in &report.extra_in_config {
            println!("  + {}", f);
        }
    }

    if !report.missing_in_config.is_empty() {
        bail!(
            "{} SC mods missing from PackConfig",
            report.missing_in_config.len()
        );
    }
    Ok(())
}

async fn run_build(
    config_path: &Path,
    storage: &Path,
    pack_version: Option<&str>,
    mirror_base: &str,
    pack_meta_path: Option<&Path>,
) -> Result<()> {
    let cfg: PackConfig = read_json(config_path)?;
    let pack_meta = pack_meta_path
        .map(load_pack_meta)
        .transpose()?
        .unwrap_or_default();
    let manifest = authoring::build_manifest(&cfg, storage, pack_version, mirror_base).await?;
    let summary = authoring::make_pack_summary(&cfg, &manifest.pack_version, &pack_meta);

    let store = Storage::new(storage.to_path_buf());
    store.save_manifest(&cfg.pack_id, &manifest).await?;
    store
        .set_latest_manifest(&cfg.pack_id, &manifest.pack_version)
        .await?;
    store.save_pack_summary(&summary).await?;
    info!(pack_version = %manifest.pack_version, "build complete");
    Ok(())
}

// ── enrichment subcommands ────────────────────────────────────────────────

fn run_enrich_mcmod(config_path: &Path, out_path: &Path, storage: &Path) -> Result<()> {
    let mut cfg: PackConfig = read_json(config_path)?;
    enrich_from_mcmod_info(&mut cfg, storage)?;
    write_pack_config(&cfg, out_path)
}

fn run_apply_role_table(config_path: &Path, table_path: &Path, out_path: &Path) -> Result<()> {
    let mut cfg: PackConfig = read_json(config_path)?;
    let table = load_role_table(table_path)?;
    let report = enrich_apply_role_table(&mut cfg, &table)?;
    if !report.unmatched_in_table.is_empty() {
        warn!(
            "role table contains {} filename(s) with no match in the pack -- check for typos: {:?}",
            report.unmatched_in_table.len(),
            report.unmatched_in_table,
        );
    }
    write_pack_config(&cfg, out_path)
}

fn run_infer_requires(config_path: &Path, out_path: &Path, storage: &Path) -> Result<()> {
    let mut cfg: PackConfig = read_json(config_path)?;
    let report = infer_requires_from_mcmod_info(&mut cfg, storage)?;
    if !report.edges_skipped_unresolved.is_empty() {
        warn!(
            "{} dependency edge(s) skipped -- modid not found among sibling mods (sample: {:?})",
            report.edges_skipped_unresolved.len(),
            report
                .edges_skipped_unresolved
                .iter()
                .take(5)
                .collect::<Vec<_>>(),
        );
    }
    write_pack_config(&cfg, out_path)
}

fn write_pack_config(cfg: &PackConfig, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    let pretty = serde_json::to_string_pretty(cfg)?;
    fs::write(path, pretty).with_context(|| format!("writing {}", path.display()))
}

async fn run_apply_curator(
    config_path: &Path,
    curator_path: &Path,
    out_path: &Path,
    storage: &Path,
    mc_version_override: Option<&str>,
) -> Result<()> {
    let mut cfg: PackConfig = read_json(config_path)?;
    let curator = load_curator(curator_path)?;
    let mc_version = mc_version_override
        .map(str::to_string)
        .unwrap_or_else(|| cfg.minecraft_version.clone());
    let modrinth = Modrinth::new()?;
    enrich_apply_curator(&mut cfg, &curator, storage, &modrinth, &mc_version).await?;
    write_pack_config(&cfg, out_path)
}

async fn run_upload_static(
    pack_id: &str,
    dir: &Path,
    mirror_base: &str,
    token_file: &Path,
    skip: &[String],
) -> Result<()> {
    let token = fs::read_to_string(token_file)
        .with_context(|| format!("reading admin token from {}", token_file.display()))?
        .trim()
        .to_string();
    if token.is_empty() {
        bail!("admin token file {} is empty", token_file.display());
    }
    if !dir.is_dir() {
        bail!("upload source {} is not a directory", dir.display());
    }
    let client = reqwest::Client::builder()
        .user_agent("Kitty-Hivens/smrt-pack")
        .build()
        .context("building reqwest client")?;

    let mut uploaded = 0usize;
    let mut skipped = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();

    walk_files_for_upload(
        dir,
        dir,
        skip,
        &mut |rel_path, abs_path| {
            // Path::join with leading separator on Linux silently drops
            // the prefix; explicit format keeps the URL well-formed.
            let url = static_upload_url(mirror_base, pack_id, &rel_path);
            info!(rel = %rel_path, "uploading");
            let body =
                fs::read(abs_path).with_context(|| format!("reading {}", abs_path.display()))?;
            let resp = futures_block_on(async {
                client.put(&url).bearer_auth(&token).body(body).send().await
            });
            match resp {
                Ok(r) if r.status().is_success() => {
                    uploaded += 1;
                }
                Ok(r) => {
                    failed.push((rel_path.clone(), format!("HTTP {}", r.status())));
                }
                Err(e) => {
                    failed.push((rel_path.clone(), e.to_string()));
                }
            }
            Ok(())
        },
        &mut skipped,
    )?;

    if !failed.is_empty() {
        warn!(
            "{} upload(s) failed (sample: {:?})",
            failed.len(),
            failed.iter().take(5).collect::<Vec<_>>()
        );
    }
    info!(
        uploaded,
        skipped,
        failed = failed.len(),
        "upload-static complete"
    );
    if !failed.is_empty() {
        bail!(
            "{} of {} uploads failed",
            failed.len(),
            uploaded + failed.len()
        );
    }
    Ok(())
}

fn walk_files_for_upload(
    root: &Path,
    here: &Path,
    skip: &[String],
    upload: &mut dyn FnMut(String, &Path) -> Result<()>,
    skipped: &mut usize,
) -> Result<()> {
    let entries = fs::read_dir(here).with_context(|| format!("read_dir {}", here.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            walk_files_for_upload(root, &path, skip, upload, skipped)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let rel = path.strip_prefix(root).with_context(|| {
            format!("relativizing {} against {}", path.display(), root.display())
        })?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if skip
            .iter()
            .any(|s| rel_str.starts_with(s) || rel_str.contains(s))
        {
            *skipped += 1;
            continue;
        }
        upload(rel_str, &path)?;
    }
    Ok(())
}

fn static_upload_url(base: &str, pack_id: &str, rel_path: &str) -> String {
    use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
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
        .add(b'&')
        .add(b'=')
        .add(b'+');
    let base = base.trim_end_matches('/');
    let pack_enc = utf8_percent_encode(pack_id, SET).to_string();
    let rel_enc = rel_path
        .split('/')
        .map(|seg| utf8_percent_encode(seg, SET).to_string())
        .collect::<Vec<_>>()
        .join("/");
    format!("{base}/v1/admin/packs/{pack_enc}/static/{rel_enc}")
}

/// Bridge sync callback world into async reqwest. The upload walk is
/// already linear (one PUT at a time -- mirror upload bandwidth is
/// the bottleneck, parallelism gains are marginal and concurrency
/// makes failure reports harder to read), so wrapping each call in
/// a runtime block_on is fine for this tool's profile.
fn futures_block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Handle::current().block_on(f)
}

// ── misc ───────────────────────────────────────────────────────────────────

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}
