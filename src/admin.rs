use crate::error::ApiError;
use crate::state::AppState;
use crate::types::*;
use axum::body::Bytes;
use axum::extract::{Path, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::Response;
use axum::routing::{delete, post, put};
use axum::{Json, Router};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/admin/servers", post(save_server))
        .route("/v1/admin/servers/:server_id", delete(delete_server))
        .route(
            "/v1/admin/cache/:prefix/:filename",
            put(put_cache_jar).delete(delete_cache_jar),
        )
        .route("/v1/admin/featured", post(save_featured))
        .layer(from_fn_with_state(state.clone(), require_admin_token))
        .with_state(state)
}

// ── auth middleware ────────────────────────────────────────────────────────

async fn require_admin_token(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // If no admin token is configured, every admin call is unauthorized --
    // refusing by default avoids accidental open-write on misconfigured
    // deployments.
    let expected = state
        .config
        .admin_token
        .as_deref()
        .ok_or(ApiError::Unauthorized)?;

    let presented = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthorized)?;

    if constant_time_eq(expected.as_bytes(), presented.as_bytes()) {
        Ok(next.run(req).await)
    } else {
        Err(ApiError::Unauthorized)
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
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

async fn save_featured(
    State(state): State<AppState>,
    Json(featured): Json<Featured>,
) -> Result<(StatusCode, Json<Featured>), ApiError> {
    state.storage.save_featured(&featured).await?;
    Ok((StatusCode::CREATED, Json(featured)))
}

// ── helpers ────────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct PutCacheResponse {
    schema_version: u32,
    sha1: String,
    size_bytes: u64,
}
