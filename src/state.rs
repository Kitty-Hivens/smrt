use crate::config::Config;
use crate::storage::Storage;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<Storage>,
    pub config: Arc<Config>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let storage = Arc::new(Storage::new(config.storage_dir.clone()));
        Self {
            storage,
            config: Arc::new(config),
        }
    }
}
