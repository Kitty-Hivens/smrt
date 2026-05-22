use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

pub fn router() -> Router {
    Router::new().route("/v1/health", get(health))
}

#[derive(Serialize)]
struct Health {
    schema_version: u32,
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        schema_version: 1,
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}
