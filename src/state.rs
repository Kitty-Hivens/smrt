use crate::authoring::{HarvestScheduler, Modrinth};
use crate::config::Config;
use crate::http::session::SessionStore;
use crate::jobs::JobRegistry;
use crate::registry::Registry;
use crate::storage::Storage;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<Storage>,
    pub config: Arc<Config>,
    pub jobs: Arc<JobRegistry>,
    /// One shared Modrinth client (pooled connections) for the admin proxy
    /// handlers, instead of a fresh TLS handshake per request.
    pub modrinth: Arc<Modrinth>,
    /// Mod-identity registry (embedded SQLite under the storage root).
    pub registry: Arc<Registry>,
    /// Coalescing background harvester. Construction only wires the deps; call
    /// `harvest.clone().spawn()` once after the runtime is up to start it.
    pub harvest: Arc<HarvestScheduler>,
    /// Server-side panel sessions (opaque cookie id -> GitHub identity + role).
    pub sessions: Arc<SessionStore>,
}

impl AppState {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        // the registry db lives next to cache/ + packs/; the parent must exist
        // before SQLite can create the file.
        std::fs::create_dir_all(&config.storage_dir).ok();
        let storage = Arc::new(Storage::new(config.storage_dir.clone()));
        let registry = Arc::new(Registry::open(config.storage_dir.join("registry.db"))?);
        let modrinth = Arc::new(Modrinth::new()?);
        let harvest = HarvestScheduler::new(storage.clone(), modrinth.clone(), registry.clone());
        Ok(Self {
            storage,
            modrinth,
            registry,
            harvest,
            config: Arc::new(config),
            jobs: Arc::new(JobRegistry::default()),
            sessions: Arc::new(SessionStore::default()),
        })
    }
}
