use axum::Router;
use smrt::{admin, config, routes, state};
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
    let state = state::AppState::new(cfg);
    let app = Router::new()
        .merge(routes::router(state.clone()))
        .merge(admin::router(state))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
