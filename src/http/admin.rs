use super::ApiError;
use crate::authoring::{Curator, ValidateReport, merge_curator, modrinth, parse_curator, validate};
use crate::domain::*;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use sha1::{Digest, Sha1};
use std::collections::HashMap;

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
        .route("/v1/admin/packs/:pack_id/static", get(list_pack_static))
        .route("/v1/admin/packs", get(list_authoring_packs))
        .route(
            "/v1/admin/packs/:pack_id/config",
            get(get_pack_config).put(put_pack_config),
        )
        .route(
            "/v1/admin/packs/:pack_id/curator",
            get(get_pack_curator).put(put_pack_curator),
        )
        .route(
            "/v1/admin/packs/:pack_id/curator/structured",
            get(get_pack_curator_structured).put(put_pack_curator_structured),
        )
        .route("/v1/admin/featured", post(save_featured))
        .route("/v1/admin/packs/:pack_id/validate", post(validate_pack))
        .route("/v1/admin/cache/removed", get(list_removed))
        .route("/v1/admin/cache/inventory", get(list_cache_usage))
        .route("/v1/admin/cache/github", post(ingest_github))
        .route("/v1/admin/modrinth/search", get(modrinth_search))
        .route("/v1/admin/modrinth/versions", get(modrinth_versions))
        .route("/v1/admin/modrinth/icon", get(modrinth_icon))
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
    state.harvest.poke(); // new artifact -> refresh the registry
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
    state.harvest.poke(); // artifact gone -> refresh the registry
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

async fn list_pack_static(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Result<Json<StaticListing>, ApiError> {
    let files = state.storage.list_pack_static(&pack_id).await?;
    Ok(Json(StaticListing {
        schema_version: SCHEMA_VERSION,
        pack_id,
        files,
    }))
}

async fn save_featured(
    State(state): State<AppState>,
    Json(featured): Json<Featured>,
) -> Result<(StatusCode, Json<Featured>), ApiError> {
    state.storage.save_featured(&featured).await?;
    Ok((StatusCode::CREATED, Json(featured)))
}

// ── Modrinth proxy (search-to-add) ──────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SearchQuery {
    q: String,
    mc: Option<String>,
    // Modrinth project kind: mod (default) / resourcepack / shader, so the
    // panel can browse packs for assets, not just mods.
    #[serde(rename = "type")]
    kind: Option<String>,
}

async fn modrinth_search(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<modrinth::SearchHit>>, ApiError> {
    // clamp to the Modrinth project kinds we support; unknown -> mod
    let kind = match q.kind.as_deref() {
        Some("resourcepack") => "resourcepack",
        Some("shader") => "shader",
        _ => "mod",
    };
    let hits = state
        .modrinth
        .search(&q.q, q.mc.as_deref(), kind)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(hits))
}

#[derive(serde::Deserialize)]
struct VersionsQuery {
    id: String,
    mc: Option<String>,
}

async fn modrinth_versions(
    State(state): State<AppState>,
    Query(q): Query<VersionsQuery>,
) -> Result<Json<Vec<modrinth::Version>>, ApiError> {
    let vs = state
        .modrinth
        .project_versions(&q.id, q.mc.as_deref())
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(vs))
}

#[derive(serde::Deserialize)]
struct IconQuery {
    id: String,
}

#[derive(serde::Serialize)]
struct IconResp {
    icon_url: Option<String>,
}

// Mirrors the launcher's per-project icon lookup so the preview can show the
// same icons the player will see for Modrinth-sourced mods without an explicit
// display.icon_url. The panel caches per project_id client-side.
async fn modrinth_icon(
    State(state): State<AppState>,
    Query(q): Query<IconQuery>,
) -> Result<Json<IconResp>, ApiError> {
    let icon_url = state
        .modrinth
        .project_icon(&q.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(IconResp { icon_url }))
}

// ── validate against an SC archive ───────────────────────────────────────────

// Cross-reference the saved config against an uploaded SC archive by mod
// filename. spawn_blocking: unzipping a large archive must not stall the runtime.
async fn validate_pack(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    body: Bytes,
) -> Result<Json<ValidateReport>, ApiError> {
    let cfg = state.storage.load_pack_config(&pack_id).await?;
    let report = tokio::task::spawn_blocking(move || validate(&cfg, &body))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .map_err(ApiError::Internal)?;
    Ok(Json(report))
}

// ── removed-list (takedown) ──────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct RemovedListing {
    schema_version: u32,
    removed: Vec<String>,
}

async fn list_removed(State(state): State<AppState>) -> Result<Json<RemovedListing>, ApiError> {
    let removed = state.storage.list_removed().await?;
    Ok(Json(RemovedListing {
        schema_version: SCHEMA_VERSION,
        removed,
    }))
}

// Enrich the cache inventory with where each jar is used, by reverse-indexing
// every authoring config's smrt_cache sources. Admin-only: it exposes which
// pack pulls which jar (and under what filename), which the public inventory
// must not. A jar with no uses is an orphan -- safe to take down.
async fn list_cache_usage(
    State(state): State<AppState>,
) -> Result<Json<CacheUsageListing>, ApiError> {
    let inventory = state.storage.list_cache_inventory().await?;
    let pack_ids = state.storage.list_authoring_packs().await?;

    let mut uses: HashMap<String, Vec<CacheUse>> = HashMap::new();
    for pid in pack_ids {
        let cfg = match state.storage.load_pack_config(&pid).await {
            Ok(c) => c,
            // a pack whose config is missing OR malformed just contributes no
            // uses; one unreadable config must not sink the whole listing
            Err(e) => {
                tracing::warn!(pack = %pid, error = %e, "skipping pack in cache usage");
                continue;
            }
        };
        for m in &cfg.mods {
            if let SourceDecl::SmrtCache { sha1 } = &m.source {
                uses.entry(sha1.clone()).or_default().push(CacheUse {
                    pack_id: pid.clone(),
                    filename: m.filename.clone(),
                });
            }
        }
        for a in &cfg.assets {
            if let SourceDecl::SmrtCache { sha1 } = &a.source {
                uses.entry(sha1.clone()).or_default().push(CacheUse {
                    pack_id: pid.clone(),
                    filename: a.dest.clone(),
                });
            }
        }
    }

    let entries = inventory
        .into_iter()
        .map(|e| {
            let uses = uses.remove(&e.sha1).unwrap_or_default();
            CacheUsageEntry {
                sha1: e.sha1,
                size_bytes: e.size_bytes,
                uses,
            }
        })
        .collect();
    Ok(Json(CacheUsageListing {
        schema_version: SCHEMA_VERSION,
        entries,
    }))
}

#[derive(serde::Deserialize)]
struct GithubIngest {
    repo: String,
    tag: String,
    asset: String,
}

fn safe_seg(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
}

// Fetch a GitHub release asset server-side and cache it by content hash, so a
// pack can pull a GitHub-only mod (open-smrt-network, hidemymods) as a normal
// smrt_cache source -- no new wire source type. Bounded to github.com release
// downloads: repo/tag/asset are validated path tokens, not an arbitrary URL,
// so this is not an open SSRF sink.
async fn ingest_github(
    State(state): State<AppState>,
    Json(req): Json<GithubIngest>,
) -> Result<(StatusCode, Json<PutCacheResponse>), ApiError> {
    let repo = req.repo.trim();
    let tag = req.tag.trim();
    let asset = req.asset.trim();
    let repo_ok = repo.matches('/').count() == 1 && repo.split('/').all(safe_seg);
    if !repo_ok || !safe_seg(tag) || !safe_seg(asset) {
        return Err(ApiError::BadRequest(
            "repo (owner/name), tag and asset must be plain tokens".into(),
        ));
    }
    let url = format!("https://github.com/{repo}/releases/download/{tag}/{asset}");
    let bytes = state
        .modrinth
        .fetch_bytes(&url)
        .await
        .map_err(ApiError::Internal)?;
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    let sha1 = hex::encode(hasher.finalize());
    state.storage.save_cache_jar(&sha1, &bytes).await?;
    state.harvest.poke(); // new artifact -> refresh the registry
    Ok((
        StatusCode::CREATED,
        Json(PutCacheResponse {
            schema_version: SCHEMA_VERSION,
            sha1,
            size_bytes: bytes.len() as u64,
        }),
    ))
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

async fn get_pack_curator_structured(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Result<Json<Curator>, ApiError> {
    let curator = match state.storage.load_curator_doc(&pack_id).await {
        Ok(text) => {
            parse_curator(&text).map_err(|e| ApiError::BadRequest(format!("curator.toml: {e}")))?
        }
        // A pack with no curator yet starts from defaults; a real read error
        // must surface, not silently present an empty curator.
        Err(ApiError::NotFound) => Curator::default(),
        Err(e) => return Err(e),
    };
    Ok(Json(curator))
}

async fn put_pack_curator_structured(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Json(curator): Json<Curator>,
) -> Result<StatusCode, ApiError> {
    // Merge the structured edit into the existing doc so section comments
    // survive; the raw editor stays the full-fidelity path.
    // Only a genuinely-absent curator is treated as empty. If the existing doc
    // fails to read, abort rather than merge into empty and overwrite it.
    let existing = match state.storage.load_curator_doc(&pack_id).await {
        Ok(text) => text,
        Err(ApiError::NotFound) => String::new(),
        Err(e) => return Err(e),
    };
    let merged = merge_curator(&existing, &curator).map_err(ApiError::Internal)?;
    state.storage.save_curator_doc(&pack_id, &merged).await?;
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

#[derive(serde::Serialize)]
struct StaticListing {
    schema_version: u32,
    pack_id: String,
    files: Vec<String>,
}
