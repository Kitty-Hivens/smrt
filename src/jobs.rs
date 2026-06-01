//! Build jobs with a live, tailable log. Authoring (curator + manifest build)
//! runs where the storage tree lives, so the panel triggers it over HTTP and
//! streams the log (SSE) instead of shelling out to `smrt-pack`. A job is an
//! in-memory log + status; `Notify` wakes SSE tailers on each new line.

use crate::authoring::{Modrinth, apply_curator, build_manifest, make_pack_summary, parse_curator};
use crate::config::Config;
use crate::storage::Storage;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Done,
    Failed,
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
}

impl JobRegistry {
    pub fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.jobs.lock().unwrap().get(id).cloned()
    }

    fn create(&self, kind: &'static str, pack_id: String) -> Arc<Job> {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let id = format!("{ms:013x}-{n}");
        let job = Arc::new(Job {
            id: id.clone(),
            kind,
            pack_id,
            state: Mutex::new(Inner {
                log: Vec::new(),
                status: Status::Running,
            }),
            notify: Notify::new(),
        });
        let mut map = self.jobs.lock().unwrap();
        map.insert(id, job.clone());
        // Bound memory: a single-admin session won't run many builds, but cap
        // the history anyway. Ids are zero-padded ms + counter, so lexical min
        // is the oldest.
        if map.len() > 50
            && let Some(oldest) = map.keys().min().cloned()
        {
            map.remove(&oldest);
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
    ) -> Arc<Job> {
        let job = self.create("build", pack_id);
        let handle = job.clone();
        tokio::spawn(async move {
            match run_build(&handle, &storage, &config).await {
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

/// Load the pack's authoring inputs, apply the curator chain transiently
/// (config.json stays the pre-curator source), resolve sources, and publish
/// the manifest + summary + latest pointer. Logs each step to the job.
async fn run_build(job: &Job, storage: &Storage, config: &Config) -> Result<(), String> {
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

    let curator = match storage.load_curator_doc(&pack_id).await {
        Ok(text) => Some(parse_curator(&text).map_err(|e| format!("curator.toml: {e}"))?),
        Err(_) => {
            job.line("no curator.toml -- building the config as-is");
            None
        }
    };

    let modrinth = Modrinth::new().map_err(|e| e.to_string())?;
    if let Some(c) = &curator {
        job.line("applying curator chain (enrich / roles / optional / substitute / extras)");
        let mc = cfg.minecraft_version.clone();
        apply_curator(&mut cfg, c, storage.root(), &modrinth, &mc)
            .await
            .map_err(|e| format!("curator failed: {e}"))?;
        job.line(format!(
            "after curator: {} mods, {} assets",
            cfg.mods.len(),
            cfg.assets.len()
        ));
    }

    job.line("resolving sources (Modrinth lookups + cache reads)");
    let manifest = build_manifest(&cfg, storage.root(), None, &config.mirror_base)
        .await
        .map_err(|e| format!("resolve failed: {e}"))?;
    let pack_meta = curator.as_ref().map(|c| c.pack_meta.clone()).unwrap_or_default();
    let summary = make_pack_summary(&cfg, &manifest.pack_version, &pack_meta);

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
    job.line(format!("build complete: {pack_id} is now {}", manifest.pack_version));
    Ok(())
}
