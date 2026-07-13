//! Background harvest coordinator: keeps the mod-identity registry fresh without
//! the operator ever running a harvest by hand. A `poke` is cheap and
//! non-blocking; pokes coalesce (a burst of cache writes, or a build that lands
//! many artifacts, settles into one harvest after a quiet window), and only one
//! harvest runs at a time since the worker is a single task. The manual
//! `/registry/harvest` endpoint stays as an immediate force-refresh.

use super::harvest;
use super::modrinth::Modrinth;
use crate::registry::Registry;
use crate::storage::Storage;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

/// Quiet window after the last poke before a harvest fires. Long enough that a
/// burst (dragging in a stack of jars, a build writing many mod rows) collapses
/// into a single run instead of one harvest per write.
const DEBOUNCE: Duration = Duration::from_secs(45);

pub struct HarvestScheduler {
    storage: Arc<Storage>,
    modrinth: Arc<Modrinth>,
    registry: Arc<Registry>,
    wake: Notify,
}

impl HarvestScheduler {
    pub fn new(
        storage: Arc<Storage>,
        modrinth: Arc<Modrinth>,
        registry: Arc<Registry>,
    ) -> Arc<Self> {
        Arc::new(Self {
            storage,
            modrinth,
            registry,
            wake: Notify::new(),
        })
    }

    /// Request a harvest soon. Non-blocking; safe to call from any handler after
    /// a change that the registry should reflect (a build, a cache write). A
    /// stored wake the worker drains -- repeated pokes before it fires are one.
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    /// Spawn the worker loop and request an initial harvest so the registry is
    /// built/refreshed shortly after startup. Call once, after the runtime is up.
    pub fn spawn(self: Arc<Self>) {
        self.wake.notify_one(); // refresh on boot
        tokio::spawn(async move {
            loop {
                // wait for a poke, then settle: each further poke restarts the
                // window, so a whole burst coalesces into one run
                self.wake.notified().await;
                loop {
                    tokio::select! {
                        _ = self.wake.notified() => continue,
                        _ = tokio::time::sleep(DEBOUNCE) => break,
                    }
                }
                // pokes arriving during the run leave a wake for the next pass --
                // those are genuinely-newer changes, worth a follow-up harvest.
                // Run in a child task so a panic deep in harvest is isolated and
                // logged instead of killing this loop and silently stopping all
                // auto-harvests for the rest of the process.
                let (storage, modrinth, registry) = (
                    self.storage.clone(),
                    self.modrinth.clone(),
                    self.registry.clone(),
                );
                let run = tokio::spawn(async move {
                    harvest::run_harvest(&storage, &modrinth, registry).await
                });
                match run.await {
                    Ok(Ok(rep)) => tracing::info!(
                        jars = rep.jars_scanned,
                        mods = rep.mods,
                        versions = rep.mod_versions,
                        builds = rep.builds,
                        inferred_requires = rep.inferred_requires,
                        inferred_optional = rep.inferred_optional,
                        sides = rep.sides_derived,
                        "auto-harvest complete"
                    ),
                    Ok(Err(e)) => tracing::warn!(error = %e, "auto-harvest failed"),
                    Err(join) => tracing::error!(error = %join, "auto-harvest task crashed"),
                }
            }
        });
    }
}
