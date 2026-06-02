use crate::authoring::Modrinth;
use crate::config::Config;
use crate::jobs::JobRegistry;
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
}

impl AppState {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let storage = Arc::new(Storage::new(config.storage_dir.clone()));
        Ok(Self {
            storage,
            config: Arc::new(config),
            jobs: Arc::new(JobRegistry::default()),
            modrinth: Arc::new(Modrinth::new()?),
        })
    }
}
