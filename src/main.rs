use smrt::{config, http, state};
use tower_http::trace::TraceLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "smrt=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cfg = config::Config::from_env()?;
    tracing::info!(
        bind = %cfg.bind_addr,
        storage = %cfg.storage_dir.display(),
        admin_token_set = cfg.admin_token.is_some(),
        "smrt starting"
    );

    let bind_addr = cfg.bind_addr;
    let state = state::AppState::new(cfg)?;
    // Jobs that were running when the previous process died can never finish:
    // mark their snapshots failed so pollers learn the truth, and bound the
    // archive while at it.
    match state.storage.sweep_job_snapshots(200).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(interrupted = n, "marked orphaned job snapshots failed"),
        Err(e) => tracing::warn!(error = %e, "job snapshot sweep failed"),
    }
    // start the coalescing background harvester (refreshes on boot + on changes)
    state.harvest.clone().spawn();
    let app = http::router(state).layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
