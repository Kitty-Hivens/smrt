use super::ApiError;
use crate::authoring::parse_curator;
use crate::domain::*;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};

// Mod jars and curated assets routinely run 5-50 MB. Axum's 2 MiB default
// trips every realistic upload; the nginx layer is already gated at 100 MB
// and the admin token is the actual authorization boundary, so a generous
// per-request cap here just avoids breaking legitimate uploads.
const ADMIN_BODY_LIMIT: usize = 256 * 1024 * 1024;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/admin/servers", post(save_server))
        .route("/v1/admin/servers/:server_id", delete(delete_server))
        .route(
            "/v1/admin/cache/:prefix/:filename",
            put(put_cache_jar).delete(delete_cache_jar),
        )
        .route(
            "/v1/admin/packs/:pack_id/static/*rel_path",
            put(put_pack_static).delete(delete_pack_static),
        )
        .route("/v1/admin/packs", get(list_authoring_packs))
        .route(
            "/v1/admin/packs/:pack_id/config",
            get(get_pack_config).put(put_pack_config),
        )
        .route(
            "/v1/admin/packs/:pack_id/curator",
            get(get_pack_curator).put(put_pack_curator),
        )
        .route("/v1/admin/featured", post(save_featured))
        .layer(DefaultBodyLimit::max(ADMIN_BODY_LIMIT))
        .layer(from_fn_with_state(state.clone(), super::auth::require_auth))
        .with_state(state)
}

// ── handlers ───────────────────────────────────────────────────────────────

async fn save_server(
    State(state): State<AppState>,
    Json(entry): Json<ServerEntry>,
) -> Result<(StatusCode, Json<ServerEntry>), ApiError> {
    state.storage.save_server(&entry).await?;
    Ok((StatusCode::CREATED, Json(entry)))
}

async fn delete_server(
    State(state): State<AppState>,
    Path(server_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.storage.delete_server(&server_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_cache_jar(
    State(state): State<AppState>,
    Path((prefix, filename)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<PutCacheResponse>), ApiError> {
    let sha1 = filename
        .strip_suffix(".jar")
        .ok_or_else(|| ApiError::BadRequest("cache path must end in .jar".into()))?;
    if !sha1.starts_with(&prefix) {
        return Err(ApiError::BadRequest("prefix does not match sha1".into()));
    }
    state.storage.save_cache_jar(sha1, &body).await?;
    Ok((
        StatusCode::CREATED,
        Json(PutCacheResponse {
            schema_version: SCHEMA_VERSION,
            sha1: sha1.to_string(),
            size_bytes: body.len() as u64,
        }),
    ))
}

async fn delete_cache_jar(
    State(state): State<AppState>,
    Path((prefix, filename)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sha1 = filename
        .strip_suffix(".jar")
        .ok_or_else(|| ApiError::BadRequest("cache path must end in .jar".into()))?;
    if !sha1.starts_with(&prefix) {
        return Err(ApiError::BadRequest("prefix does not match sha1".into()));
    }
    state.storage.delete_cache_jar(sha1).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_pack_static(
    State(state): State<AppState>,
    Path((pack_id, rel_path)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<PutStaticResponse>), ApiError> {
    state
        .storage
        .save_pack_static(&pack_id, &rel_path, &body)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(PutStaticResponse {
            schema_version: SCHEMA_VERSION,
            pack_id,
            rel_path,
            size_bytes: body.len() as u64,
        }),
    ))
}

async fn delete_pack_static(
    State(state): State<AppState>,
    Path((pack_id, rel_path)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    state
        .storage
        .delete_pack_static(&pack_id, &rel_path)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn save_featured(
    State(state): State<AppState>,
    Json(featured): Json<Featured>,
) -> Result<(StatusCode, Json<Featured>), ApiError> {
    state.storage.save_featured(&featured).await?;
    Ok((StatusCode::CREATED, Json(featured)))
}

// ── authoring inputs ───────────────────────────────────────────────────────

async fn list_authoring_packs(
    State(state): State<AppState>,
) -> Result<Json<AuthoringPacksListing>, ApiError> {
    let packs = state.storage.list_authoring_packs().await?;
    Ok(Json(AuthoringPacksListing {
        schema_version: SCHEMA_VERSION,
        packs,
    }))
}

async fn get_pack_config(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Result<Json<PackConfig>, ApiError> {
    Ok(Json(state.storage.load_pack_config(&pack_id).await?))
}

async fn put_pack_config(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Json(cfg): Json<PackConfig>,
) -> Result<(StatusCode, Json<PackConfig>), ApiError> {
    // The path id is authoritative; reject a body that disagrees so a
    // mis-targeted PUT can't write one pack's config under another's id.
    if cfg.pack_id != pack_id {
        return Err(ApiError::BadRequest(format!(
            "body pack_id {:?} does not match path {:?}",
            cfg.pack_id, pack_id
        )));
    }
    state.storage.save_pack_config(&pack_id, &cfg).await?;
    Ok((StatusCode::CREATED, Json(cfg)))
}

async fn get_pack_curator(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Result<Response, ApiError> {
    let text = state.storage.load_curator_doc(&pack_id).await?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/toml; charset=utf-8")],
        text,
    )
        .into_response())
}

async fn put_pack_curator(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    // Validate the doc parses as a Curator before persisting, so a later
    // build never trips over a doc the panel accepted. The raw text is
    // stored verbatim (comments preserved); only the shape is checked here.
    parse_curator(&body)
        .map_err(|e| ApiError::BadRequest(format!("curator.toml does not parse: {e}")))?;
    state.storage.save_curator_doc(&pack_id, &body).await?;
    Ok(StatusCode::CREATED)
}

// ── helpers ────────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct PutCacheResponse {
    schema_version: u32,
    sha1: String,
    size_bytes: u64,
}

#[derive(serde::Serialize)]
struct PutStaticResponse {
    schema_version: u32,
    pack_id: String,
    rel_path: String,
    size_bytes: u64,
}
