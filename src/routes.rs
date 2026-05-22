use crate::error::ApiError;
use crate::state::AppState;
use crate::types::*;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use tokio_util::io::ReaderStream;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/packs", get(list_packs))
        .route("/v1/packs/:pack_id", get(get_pack_summary))
        // Static segments win over dynamic in axum 0.7, so order does not
        // matter for /manifest/versions vs /manifest/:version, but keeping
        // the more specific routes first matches the spec ordering.
        .route("/v1/packs/:pack_id/manifest", get(get_latest_manifest))
        .route("/v1/packs/:pack_id/manifest/versions", get(list_manifest_versions))
        .route("/v1/packs/:pack_id/manifest/:version", get(get_manifest_version))
        .route("/v1/packs/:pack_id/extras/:version_zip", get(get_pack_extras))
        .route("/v1/servers", get(list_servers))
        .route("/v1/servers/:server_id", get(get_server))
        .route("/v1/featured", get(get_featured))
        .route("/v1/cache/:prefix/:filename", get(get_cache_jar))
        .route("/v1/cache/inventory", get(get_cache_inventory))
        .with_state(state)
}

// ── /v1/health ─────────────────────────────────────────────────────────────

async fn health() -> Json<Health> {
    Json(Health {
        schema_version: SCHEMA_VERSION,
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

// ── /v1/packs ──────────────────────────────────────────────────────────────

async fn list_packs(State(state): State<AppState>) -> Result<Json<PackListing>, ApiError> {
    let packs = state.storage.list_pack_summaries().await?;
    Ok(Json(PackListing {
        schema_version: SCHEMA_VERSION,
        generated_at: now_rfc3339(),
        packs,
    }))
}

async fn get_pack_summary(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Result<Json<PackSummary>, ApiError> {
    Ok(Json(state.storage.load_pack_summary(&pack_id).await?))
}

async fn get_latest_manifest(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Result<Json<PackManifest>, ApiError> {
    Ok(Json(state.storage.load_latest_manifest(&pack_id).await?))
}

async fn get_manifest_version(
    State(state): State<AppState>,
    Path((pack_id, version)): Path<(String, String)>,
) -> Result<Json<PackManifest>, ApiError> {
    Ok(Json(state.storage.load_manifest_version(&pack_id, &version).await?))
}

async fn list_manifest_versions(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Result<Json<ManifestVersionsListing>, ApiError> {
    let versions = state.storage.list_manifest_versions(&pack_id).await?;
    Ok(Json(ManifestVersionsListing {
        schema_version: SCHEMA_VERSION,
        pack_id,
        versions,
    }))
}

async fn get_pack_extras(
    State(state): State<AppState>,
    Path((pack_id, version_zip)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let version = version_zip
        .strip_suffix(".zip")
        .ok_or_else(|| ApiError::BadRequest("extras path must end in .zip".into()))?;
    let path = state.storage.pack_extras_path(&pack_id, version).await?;
    serve_file(&path, "application/zip").await
}

// ── /v1/servers ────────────────────────────────────────────────────────────

async fn list_servers(State(state): State<AppState>) -> Result<Json<ServerListing>, ApiError> {
    let servers = state.storage.list_servers().await?;
    Ok(Json(ServerListing {
        schema_version: SCHEMA_VERSION,
        generated_at: now_rfc3339(),
        servers,
    }))
}

async fn get_server(
    State(state): State<AppState>,
    Path(server_id): Path<String>,
) -> Result<Json<ServerEntry>, ApiError> {
    Ok(Json(state.storage.load_server(&server_id).await?))
}

// ── /v1/featured ───────────────────────────────────────────────────────────

async fn get_featured(State(state): State<AppState>) -> Result<Json<Featured>, ApiError> {
    Ok(Json(state.storage.load_featured().await?))
}

// ── /v1/cache ──────────────────────────────────────────────────────────────

async fn get_cache_jar(
    State(state): State<AppState>,
    Path((prefix, filename)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let sha1 = filename
        .strip_suffix(".jar")
        .ok_or_else(|| ApiError::BadRequest("cache path must end in .jar".into()))?;
    let path = state.storage.cache_jar_path(&prefix, sha1)?;
    if tokio::fs::metadata(&path).await.is_err() {
        return Err(ApiError::NotFound);
    }
    serve_file(&path, "application/java-archive").await
}

async fn get_cache_inventory(
    State(state): State<AppState>,
) -> Result<Json<CacheInventory>, ApiError> {
    let entries = state.storage.list_cache_inventory().await?;
    Ok(Json(CacheInventory {
        schema_version: SCHEMA_VERSION,
        generated_at: now_rfc3339(),
        entries,
    }))
}

// ── helpers ────────────────────────────────────────────────────────────────

async fn serve_file(path: &std::path::Path, content_type: &str) -> Result<Response, ApiError> {
    let file = tokio::fs::File::open(path).await.map_err(|_| ApiError::NotFound)?;
    let meta = file.metadata().await.map_err(|e| ApiError::Internal(e.into()))?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (header::CONTENT_LENGTH, meta.len().to_string()),
        ],
        body,
    )
        .into_response())
}

fn now_rfc3339() -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}
