//! Build jobs with a live, tailable log. Authoring (enrichment passes +
//! manifest build) runs where the storage tree lives, so the panel triggers it
//! over HTTP and streams the log (SSE) instead of shelling out to `smrt-pack`.
//! A job is an in-memory log + status; `Notify` wakes SSE tailers on each new
//! line.

use crate::authoring::{
    self, BootstrapArgs, HarvestScheduler, build_manifest, enrich_from_mcmod_info,
    infer_requires_from_mcmod_info, make_pack_summary,
};
use crate::config::Config;
use crate::domain::{PackManifest, PackSummary};
use crate::storage::Storage;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Done,
    Failed,
}

/// What a dry-run build computes without publishing: the resolved manifest the
/// launcher would download, plus the summary card it would show. Stashed on the
/// job so the panel can render a launcher-faithful preview and diff it against
/// the currently-published manifest.
#[derive(Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct DryRun {
    pub manifest: PackManifest,
    pub summary: PackSummary,
}

pub struct Job {
    pub id: String,
    pub kind: &'static str,
    pub pack_id: String,
    state: Mutex<Inner>,
    notify: Notify,
}

struct Inner {
    log: Vec<String>,
    status: Status,
    result: Option<DryRun>,
}

impl Job {
    fn line(&self, text: impl Into<String>) {
        self.state.lock().unwrap().log.push(text.into());
        self.notify.notify_waiters();
    }

    fn finish(&self, status: Status) {
        self.state.lock().unwrap().status = status;
        self.notify.notify_waiters();
    }

    fn set_result(&self, result: DryRun) {
        self.state.lock().unwrap().result = Some(result);
    }

    /// The dry-run result, if this was a preview build that has produced one.
    pub fn result(&self) -> Option<DryRun> {
        self.state.lock().unwrap().result.clone()
    }

    pub fn status(&self) -> Status {
        self.state.lock().unwrap().status
    }

    /// Log lines from index `from` onward, plus the current status. SSE tailers
    /// track how many lines they've sent and re-read from there, so no line is
    /// ever dropped regardless of `Notify` timing.
    pub fn since(&self, from: usize) -> (Vec<String>, Status) {
        let st = self.state.lock().unwrap();
        let tail = st.log.get(from..).map(|s| s.to_vec()).unwrap_or_default();
        (tail, st.status)
    }

    pub async fn wait(&self) {
        self.notify.notified().await;
    }
}

#[derive(Default)]
pub struct JobRegistry {
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    counter: AtomicU64,
    /// Per-pack lock so two real (publishing) builds of the same pack can't
    /// interleave save_manifest / set_latest / summary.
    pack_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl JobRegistry {
    pub fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.jobs.lock().unwrap().get(id).cloned()
    }

    fn pack_lock(&self, pack_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.pack_locks
            .lock()
            .unwrap()
            .entry(pack_id.to_string())
            .or_default()
            .clone()
    }

    fn create(&self, kind: &'static str, pack_id: String) -> Arc<Job> {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let id = format!("{ms:013x}-{n:08}");
        let job = Arc::new(Job {
            id: id.clone(),
            kind,
            pack_id,
            state: Mutex::new(Inner {
                log: Vec::new(),
                status: Status::Running,
                result: None,
            }),
            notify: Notify::new(),
        });
        let mut map = self.jobs.lock().unwrap();
        map.insert(id, job.clone());
        // Bound memory, but never evict a job a client may still be tailing:
        // drop the oldest FINISHED job. Ids are zero-padded (ms + counter) so
        // lexical min is the oldest.
        if map.len() > 50 {
            let victim = map
                .values()
                .filter(|j| j.status() != Status::Running)
                .map(|j| j.id.clone())
                .min();
            if let Some(victim) = victim {
                map.remove(&victim);
            }
        }
        job
    }

    /// Create a build job and run it on a background task. Returns immediately
    /// with the job handle so the caller can hand back a job id.
    pub fn spawn_build(
        &self,
        pack_id: String,
        storage: Arc<Storage>,
        config: Arc<Config>,
        dry_run: bool,
        pack_version: Option<String>,
        harvest: Option<Arc<HarvestScheduler>>,
    ) -> Arc<Job> {
        let job = self.create(if dry_run { "preview" } else { "build" }, pack_id);
        let handle = job.clone();
        // Real builds of the same pack are serialized; a dry run never publishes
        // so it takes no lock.
        let lock = (!dry_run).then(|| self.pack_lock(&job.pack_id));
        tokio::spawn(async move {
            let _guard = match lock {
                Some(l) => Some(l.lock_owned().await),
                None => None,
            };
            match run_build(&handle, &storage, &config, dry_run, pack_version).await {
                Ok(()) => {
                    handle.finish(Status::Done);
                    // a published build added a build + its mods to harvest -- a
                    // dry run published nothing, so it needn't refresh
                    if !dry_run && let Some(h) = harvest {
                        h.poke();
                    }
                }
                Err(e) => {
                    handle.line(format!("failed: {e}"));
                    handle.finish(Status::Failed);
                }
            }
        });
        job
    }

    /// Create a bootstrap job: stage an SC archive into the cache + static and
    /// write the starter authoring config, on a background task.
    pub fn spawn_bootstrap(
        &self,
        pack_id: String,
        args: BootstrapArgs,
        archive: Vec<u8>,
        storage: Arc<Storage>,
    ) -> Arc<Job> {
        let job = self.create("bootstrap", pack_id);
        let handle = job.clone();
        tokio::spawn(async move {
            match run_bootstrap(&handle, args, archive, &storage).await {
                Ok(()) => handle.finish(Status::Done),
                Err(e) => {
                    handle.line(format!("failed: {e}"));
                    handle.finish(Status::Failed);
                }
            }
        });
        job
    }
}

/// Load the pack's authoring inputs, run the build enrichment passes
/// transiently (config.json stays the source on disk), resolve sources, and
/// publish the manifest + summary + latest pointer. Logs each step to the job.
async fn run_build(
    job: &Job,
    storage: &Storage,
    config: &Config,
    dry_run: bool,
    pack_version: Option<String>,
) -> Result<(), String> {
    let pack_id = job.pack_id.clone();
    job.line(format!("build {pack_id}: loading authoring inputs"));
    let mut cfg = storage
        .load_pack_config(&pack_id)
        .await
        .map_err(|e| format!("no authoring config: {e}"))?;
    job.line(format!(
        "config: {} mods, {} assets",
        cfg.mods.len(),
        cfg.assets.len()
    ));

    // Enrichment passes run on a transient copy of the config: fill display
    // metadata from each cache jar's mcmod.info, then infer the requires graph.
    job.line("running enrichment passes (enrich-mcmod / infer-requires)");
    enrich_from_mcmod_info(&mut cfg, storage.root())
        .map_err(|e| format!("enrich-mcmod failed: {e}"))?;
    infer_requires_from_mcmod_info(&mut cfg, storage.root())
        .map_err(|e| format!("infer-requires failed: {e}"))?;

    job.line("resolving sources (Modrinth lookups + cache reads)");
    let manifest = build_manifest(
        &cfg,
        storage.root(),
        pack_version.as_deref(),
        &config.mirror_base,
    )
    .await
    .map_err(|e| format!("resolve failed: {e}"))?;
    let summary = make_pack_summary(&cfg, &manifest.pack_version);

    if dry_run {
        job.line(format!(
            "dry run: resolved {} ({} mods, {} assets) -- not publishing",
            manifest.pack_version,
            manifest.mods.len(),
            manifest.assets.len()
        ));
        job.set_result(DryRun { manifest, summary });
        return Ok(());
    }

    job.line(format!(
        "writing manifest {} ({} mods, {} assets)",
        manifest.pack_version,
        manifest.mods.len(),
        manifest.assets.len()
    ));
    storage
        .save_manifest(&pack_id, &manifest)
        .await
        .map_err(|e| e.to_string())?;
    storage
        .set_latest_manifest(&pack_id, &manifest.pack_version)
        .await
        .map_err(|e| e.to_string())?;
    storage
        .save_pack_summary(&summary)
        .await
        .map_err(|e| e.to_string())?;
    job.line(format!(
        "build complete: {pack_id} is now {}",
        manifest.pack_version
    ));
    Ok(())
}

async fn run_bootstrap(
    job: &Job,
    args: BootstrapArgs,
    archive: Vec<u8>,
    storage: &Storage,
) -> Result<(), String> {
    let pack_id = args.pack_id.clone();
    job.line(format!(
        "bootstrap {pack_id}: reading SC archive ({} bytes)",
        archive.len()
    ));
    let cfg = authoring::bootstrap(args, archive)
        .await
        .map_err(|e| e.to_string())?;
    job.line(format!(
        "discovered {} mods, {} assets; staged into cache + static",
        cfg.mods.len(),
        cfg.assets.len()
    ));
    storage
        .save_pack_config(&pack_id, &cfg)
        .await
        .map_err(|e| e.to_string())?;
    job.line(format!(
        "wrote authoring config for {pack_id} -- ready to edit + build"
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DeclaredMod, LoaderSpec, PackConfig, PackTier, SourceDecl, Visibility};
    use sha1::{Digest, Sha1};
    use std::time::Duration;

    fn sha1_of(bytes: &[u8]) -> String {
        let mut hasher = Sha1::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    fn cfg_with_cache_mod(sha1: &str) -> PackConfig {
        PackConfig {
            pack_id: "Test".into(),
            display_name: "Test Pack".into(),
            tagline: "t".into(),
            minecraft_version: "1.12.2".into(),
            loader: LoaderSpec {
                name: "forge".into(),
                version: "14.23.5.2860".into(),
            },
            java_major: 8,
            version: None,
            tags: vec![],
            featured: false,
            mods: vec![DeclaredMod {
                filename: "Test.jar".into(),
                default_enabled: true,
                source: SourceDecl::SmrtCache { sha1: sha1.into() },
                display: None,
                slug: None,
            }],
            assets: vec![],
            pack_meta: Default::default(),
            owner: 211033194,
            tier: PackTier::Official,
            visibility: Visibility::Published,
            fork_of: None,
        }
    }

    fn test_config(storage_dir: std::path::PathBuf) -> Config {
        Config {
            bind_addr: "127.0.0.1:9000".parse().unwrap(),
            storage_dir,
            admin_token: None,
            cookie_secure: false,
            mirror_base: "https://test.example".into(),
            github_client_id: None,
            github_client_secret: None,
            admin_github_uids: Vec::new(),
            debug_token: None,
            debug_github_uids: Vec::new(),
        }
    }

    async fn await_finish(job: &Job) -> Status {
        for _ in 0..300 {
            let (_, status) = job.since(0);
            if status != Status::Running {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("job did not finish within timeout");
    }

    #[tokio::test]
    async fn dry_run_computes_manifest_without_publishing() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Arc::new(Storage::new(tmp.path().to_path_buf()));
        let bytes = b"fake jar bytes";
        let sha1 = sha1_of(bytes);
        storage.save_cache_jar(&sha1, bytes).await.unwrap();
        storage
            .save_pack_config("Test", &cfg_with_cache_mod(&sha1))
            .await
            .unwrap();
        let config = Arc::new(test_config(tmp.path().to_path_buf()));

        let registry = JobRegistry::default();
        let job = registry.spawn_build("Test".into(), storage.clone(), config, true, None, None);
        assert_eq!(job.kind, "preview");
        assert_eq!(await_finish(&job).await, Status::Done);

        let result = job.result().expect("dry run stashes a result");
        assert_eq!(result.manifest.pack_id, "Test");
        assert_eq!(result.manifest.mods.len(), 1);
        assert_eq!(result.manifest.mods[0].filename, "Test.jar");
        assert_eq!(result.manifest.mods[0].sha1, sha1);
        assert_eq!(
            result.manifest.mods[0].size_bytes,
            b"fake jar bytes".len() as u64
        );
        assert_eq!(result.summary.display_name, "Test Pack");

        // The whole point of a dry run: nothing reaches the public surface.
        assert!(
            storage.load_latest_manifest("Test").await.is_err(),
            "dry run must not publish a latest manifest"
        );
        assert!(
            storage
                .list_manifest_versions("Test")
                .await
                .unwrap_or_default()
                .is_empty(),
            "dry run must not write a versioned manifest"
        );
    }

    #[tokio::test]
    async fn real_build_publishes_and_carries_no_dry_run_result() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Arc::new(Storage::new(tmp.path().to_path_buf()));
        let bytes = b"jar";
        let sha1 = sha1_of(bytes);
        storage.save_cache_jar(&sha1, bytes).await.unwrap();
        storage
            .save_pack_config("Test", &cfg_with_cache_mod(&sha1))
            .await
            .unwrap();
        let config = Arc::new(test_config(tmp.path().to_path_buf()));

        let registry = JobRegistry::default();
        let job = registry.spawn_build("Test".into(), storage.clone(), config, false, None, None);
        assert_eq!(job.kind, "build");
        assert_eq!(await_finish(&job).await, Status::Done);

        assert!(
            job.result().is_none(),
            "a real build stashes no preview result"
        );
        let published = storage.load_latest_manifest("Test").await.unwrap();
        assert_eq!(published.mods.len(), 1);
        assert_eq!(published.mods[0].filename, "Test.jar");
    }
}
