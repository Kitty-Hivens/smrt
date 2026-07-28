use super::ApiError;
use crate::accounts::{AuditRow, Identity, UploadRow, UserRow};
use crate::authoring::{
    ResolveReport, ValidateReport, modrinth, pack_graph, reconstruct_config, resolve_pack, validate,
};
use crate::domain::*;
use crate::registry::model::GraphData;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderName, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use sha1::{Digest, Sha1};
use std::collections::HashMap;

use super::MAX_UPLOAD_BODY;

pub fn router(state: AppState) -> Router {
    operator_router(state.clone()).merge(authoring_router(state))
}

/// Operator-only surface: the official catalog, servers, cache management, and
/// user administration. Requires the admin role (and up).
fn operator_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/servers", post(save_server))
        .route("/v1/servers/{server_id}", delete(delete_server))
        .route(
            "/v1/cache/{prefix}/{filename}",
            put(put_cache_jar).delete(delete_cache_jar),
        )
        .route("/v1/authoring/packs", get(list_authoring_packs))
        .route("/v1/authoring/summaries", get(list_all_pack_summaries))
        .route("/v1/featured", post(save_featured))
        .route("/v1/cache/removed", get(list_removed))
        .route(
            "/v1/cache/removed/{sha1}",
            post(takedown_jar).delete(restore_jar),
        )
        .route("/v1/cache/usage", get(list_cache_usage))
        .route("/v1/cache/github", post(ingest_github))
        .route("/v1/users", get(list_users))
        .route("/v1/users/{uid}/role", post(set_user_role))
        .route("/v1/uploads", get(list_uploads))
        .route("/v1/uploads/{id}/approve", post(approve_upload))
        .route("/v1/uploads/{id}/reject", post(reject_upload))
        .route("/v1/audit", get(get_audit_log))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY))
        .layer(from_fn_with_state(state.clone(), super::auth::require_auth))
        .with_state(state)
}

/// Pack-authoring surface: any signed-in member may reach it, but every
/// pack-scoped handler gates on `may_author` -- a member touches only their own
/// community packs, officials stay admin-only. The Modrinth proxy needs just a
/// session (no pack, no ownership).
fn authoring_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/authoring/packs/{pack_id}/static/{*rel_path}",
            put(put_pack_static).delete(delete_pack_static),
        )
        .route(
            "/v1/authoring/packs/{pack_id}/static",
            get(list_pack_static),
        )
        .route(
            "/v1/authoring/packs/{pack_id}/config",
            get(get_pack_config).put(put_pack_config),
        )
        .route("/v1/authoring/packs/{pack_id}", delete(delete_pack))
        .route(
            "/v1/authoring/packs/{pack_id}/visibility",
            put(set_pack_visibility),
        )
        .route(
            "/v1/authoring/packs/{pack_id}/config/revert",
            post(revert_pack_config),
        )
        .route(
            "/v1/authoring/packs/{pack_id}/duplicate",
            post(duplicate_pack),
        )
        .route(
            "/v1/authoring/packs/{pack_id}/validate",
            post(validate_pack),
        )
        .route(
            "/v1/authoring/packs/{pack_id}/dependency-preview",
            post(preview_dependencies),
        )
        .route("/v1/authoring/packs/{pack_id}/resolve", get(pack_resolve))
        .route("/v1/authoring/packs/{pack_id}/graph", get(pack_graph_view))
        .route("/v1/search/mods", get(search_mods_combined))
        .route("/v1/modrinth/search", get(modrinth_search))
        .route("/v1/modrinth/versions", get(modrinth_versions))
        .route("/v1/modrinth/icon", get(modrinth_icon))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY))
        .layer(from_fn_with_state(
            state.clone(),
            super::auth::require_session,
        ))
        .with_state(state)
}

// ── audit ────────────────────────────────────────────────────────────────────

use super::audit;

/// The recent audit trail, newest first -- the operator's "who did what" view.
async fn get_audit_log(State(state): State<AppState>) -> Result<Json<Vec<AuditRow>>, ApiError> {
    let acc = state.accounts.clone();
    let rows = tokio::task::spawn_blocking(move || acc.list_audit(200))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("audit task: {e}")))??;
    Ok(Json(rows))
}

// ── handlers ───────────────────────────────────────────────────────────────

/// Every registered user and their role, for the operator's user-management
/// view. Break-glass is excluded (it is a synthetic row, not a person).
async fn list_users(State(state): State<AppState>) -> Result<Json<Vec<UserRow>>, ApiError> {
    let acc = state.accounts.clone();
    let users = tokio::task::spawn_blocking(move || acc.list_users())
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("users task: {e}")))??;
    Ok(Json(users))
}

#[derive(serde::Deserialize)]
struct RoleReq {
    role: String,
}

/// Set a user's role (member/admin) by GitHub uid.
async fn set_user_role(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(uid): Path<i64>,
    Json(req): Json<RoleReq>,
) -> Result<StatusCode, ApiError> {
    let acc = state.accounts.clone();
    let role = req.role.clone();
    tokio::task::spawn_blocking(move || acc.set_role(uid, &role))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("role task: {e}")))?
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    audit(
        &state,
        &identity,
        "role.set",
        Some(&uid.to_string()),
        Some(&req.role),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Pending member uploads awaiting moderation, oldest first.
async fn list_uploads(State(state): State<AppState>) -> Result<Json<Vec<UploadRow>>, ApiError> {
    let acc = state.accounts.clone();
    let rows = tokio::task::spawn_blocking(move || acc.list_pending_uploads())
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("uploads task: {e}")))??;
    Ok(Json(rows))
}

/// Approve a staged upload: promote its jar into the shared cache and mark it
/// approved. The registry is poked so the new artifact is harvested.
async fn approve_upload(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let acc = state.accounts.clone();
    let upload = tokio::task::spawn_blocking(move || acc.get_upload(id))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("upload task: {e}")))??
        .ok_or(ApiError::NotFound)?;
    state.storage.promote_upload(&upload.sha1).await?;
    let acc = state.accounts.clone();
    let decided_by = identity.uid;
    tokio::task::spawn_blocking(move || {
        acc.set_upload_status(id, "approved", None, Some(decided_by))
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("status task: {e}")))??;
    state.harvest.poke();
    audit(
        &state,
        &identity,
        "upload.approve",
        Some(&upload.sha1),
        Some(&upload.pack_id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
struct RejectBody {
    note: Option<String>,
}

/// Reject a staged upload: drop its staged jar and mark it rejected, with an
/// optional moderator note the uploader sees.
async fn reject_upload(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<i64>,
    Json(body): Json<RejectBody>,
) -> Result<StatusCode, ApiError> {
    let acc = state.accounts.clone();
    let upload = tokio::task::spawn_blocking(move || acc.get_upload(id))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("upload task: {e}")))??
        .ok_or(ApiError::NotFound)?;
    state.storage.discard_upload(&upload.sha1).await?;
    let acc = state.accounts.clone();
    let note = body.note.clone();
    let decided_by = identity.uid;
    tokio::task::spawn_blocking(move || {
        acc.set_upload_status(id, "rejected", note.as_deref(), Some(decided_by))
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("status task: {e}")))??;
    audit(
        &state,
        &identity,
        "upload.reject",
        Some(&upload.sha1),
        body.note.as_deref(),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

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
    Extension(identity): Extension<Identity>,
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
    audit(&state, &identity, "cache.delete", Some(sha1), None).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Block a jar (copyright / policy): drop any cached copy and tombstone the sha1
/// so it can neither be served nor re-ingested. Deliberate and reversible via
/// `restore`; distinct from delete, which only frees bytes (#14).
async fn takedown_jar(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(sha1): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.storage.takedown(&sha1).await?;
    state.harvest.poke();
    audit(&state, &identity, "cache.takedown", Some(&sha1), None).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Lift a takedown: remove the sha1 from the removed list. The bytes are not
/// restored -- re-add the jar to recache it.
async fn restore_jar(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(sha1): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.storage.restore(&sha1).await?;
    state.harvest.poke();
    audit(&state, &identity, "cache.restore", Some(&sha1), None).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_pack_static(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path((pack_id, rel_path)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<PutStaticResponse>), ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
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
    Extension(identity): Extension<Identity>,
    Path((pack_id, rel_path)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    state
        .storage
        .delete_pack_static(&pack_id, &rel_path)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_pack_static(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
) -> Result<Json<StaticListing>, ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
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

#[derive(serde::Deserialize)]
struct ModSearchQuery {
    q: String,
    mc: Option<String>,
    loader: Option<String>,
    /// The pack being filled. Its declared mods decide whether a foreign-loader
    /// hit is carried by a bridge already in the pack or would need one added --
    /// two different answers that must not be flattened into "incompatible".
    pack: Option<String>,
    limit: Option<usize>,
}

/// One search over both places a mod can come from (#101).
///
/// Provenance is the mirror's problem, not a question to ask before the question:
/// a hit says whether the mirror holds the bytes, and the caller does not have to
/// pick a door first. Neither loader nor Minecraft version filters -- a mod
/// riding a bridge loads, and hiding it would be a lie -- they rank, and each hit
/// carries the verdict it was ranked by.
async fn search_mods_combined(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Query(q): Query<ModSearchQuery>,
) -> Result<Json<Vec<crate::authoring::ModHit>>, ApiError> {
    // The pack is optional, but reading one is authoring: gate it the same way
    // the rest of the authoring surface is.
    let present: std::collections::HashSet<String> = match &q.pack {
        Some(pack_id) => {
            if !super::auth::may_author(&identity, pack_id) {
                return Err(ApiError::Forbidden);
            }
            state
                .storage
                .load_pack_config(pack_id)
                .await
                .map(|cfg| {
                    cfg.mods
                        .iter()
                        .filter_map(|m| match &m.source {
                            SourceDecl::Modrinth { project_id, .. } => Some(project_id.clone()),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        None => Default::default(),
    };
    let cached: std::collections::HashSet<String> = state
        .storage
        .list_cache_inventory()
        .await
        .map(|inv| inv.into_iter().map(|e| e.sha1).collect())
        .unwrap_or_default();

    let ctx = crate::authoring::PackContext {
        mc: q.mc.as_deref(),
        loader: q.loader.as_deref(),
        present_projects: &present,
    };
    let hits = crate::authoring::search_mods(
        &q.q,
        &ctx,
        &cached,
        q.limit.unwrap_or(30).clamp(1, 100),
        &state.registry,
        &state.modrinth,
    )
    .await
    .map_err(ApiError::Internal)?;
    Ok(Json(hits))
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
    // An icon is cosmetic garnish -- the preview falls back to a letter avatar
    // without one. A transient upstream fault should never turn into a 500 that
    // paints the panel with errors, but it also should not vanish silently, so it
    // is logged and degraded to "no icon".
    let icon_url = match state.modrinth.project_icon(&q.id).await {
        Ok(url) => url,
        Err(e) => {
            tracing::warn!(project = %q.id, error = %e, "modrinth icon lookup failed");
            None
        }
    };
    Ok(Json(IconResp { icon_url }))
}

// ── validate against an SC archive ───────────────────────────────────────────

// Cross-reference the saved config against an uploaded SC archive by mod
// filename. spawn_blocking: unzipping a large archive must not stall the runtime.
async fn validate_pack(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
    body: Bytes,
) -> Result<Json<ValidateReport>, ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    let cfg = state.storage.load_pack_config(&pack_id).await?;
    let report = tokio::task::spawn_blocking(move || validate(&cfg, &body))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .map_err(ApiError::Internal)?;
    Ok(Json(report))
}

/// What saving this config would pull in, answered before the save.
///
/// Adding a mod landed blind: the dependencies appeared later, on save, and the
/// curator learned what came with it after the fact (#53). The plan was already
/// computed on every save; this asks for it in advance. Read-only -- it runs the
/// real fill on a copy, so the answer cannot drift from what the save does, and
/// writes nothing.
///
/// The body is the config as the editor currently holds it, not the stored one:
/// the question is about the mod just added, which by definition is not saved.
async fn preview_dependencies(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
    Json(cfg): Json<PackConfig>,
) -> Result<Json<Vec<crate::authoring::depfill::PulledPreview>>, ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    let cached: std::collections::HashSet<String> = state
        .storage
        .list_cache_inventory()
        .await
        .map(|inv| inv.into_iter().map(|e| e.sha1).collect())
        .unwrap_or_default();
    let pulled =
        crate::authoring::depfill::preview_fill(&cfg, &state.registry, &state.modrinth, &cached)
            .await
            .map_err(ApiError::Internal)?;
    Ok(Json(pulled))
}

// ── resolve against the dependency graph ─────────────────────────────────────

// Read the saved config and check it against the registry dependency graph:
// unmet hard deps, active conflicts, capability overlaps, version windows, and
// which declared mods are depended-on. Read-only -- it never edits the config,
// so required/optional stays the pack's decision. spawn_blocking: the registry
// is a synchronous SQLite handle.
async fn pack_resolve(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
) -> Result<Json<ResolveReport>, ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    let cfg = state.storage.load_pack_config(&pack_id).await?;
    let registry = state.registry.clone();
    let report = tokio::task::spawn_blocking(move || registry.with_conn(|c| resolve_pack(c, &cfg)))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("resolve task: {e}")))?
        .map_err(ApiError::Internal)?;
    Ok(Json(report))
}

/// The pack's own relation graph: its mods, wired by what the exact artifacts it
/// ships declare. The registry-wide graph answers "what does the mirror hold"; this
/// answers "does this pack hold together", which is the question being asked while
/// a pack is authored. Same shape as the registry graph, so the panel renders it
/// with the same view -- an edge that dangles here is a requirement the pack does
/// not carry.
async fn pack_graph_view(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
) -> Result<Json<GraphData>, ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    let cfg = state.storage.load_pack_config(&pack_id).await?;
    let registry = state.registry.clone();
    let graph = tokio::task::spawn_blocking(move || registry.with_conn(|c| pack_graph(c, &cfg)))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("pack graph task: {e}")))?
        .map_err(ApiError::Internal)?;
    Ok(Json(graph))
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

/// A single URL path segment for a release tag / asset name. Real GitHub release
/// filenames carry spaces, parens, commas etc, so this only rejects what would
/// change the URL's shape -- path separators, `.`/`..` traversal, control chars,
/// emptiness. Everything else is allowed and percent-encoded into the URL.
fn safe_path_seg(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains('/')
        && !s.contains('\\')
        && !s.chars().any(|c| c.is_control())
}

/// Percent-encode one URL path segment: keep the RFC 3986 unreserved set, encode
/// space and everything URL-structural so a filename with spaces/`&`/`+`/`#`
/// produces a valid, unambiguous path.
fn enc_seg(s: &str) -> String {
    use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
    const SET: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'<')
        .add(b'>')
        .add(b'?')
        .add(b'[')
        .add(b'\\')
        .add(b']')
        .add(b'^')
        .add(b'`')
        .add(b'{')
        .add(b'|')
        .add(b'}')
        .add(b'/')
        .add(b'&')
        .add(b'=')
        .add(b'+');
    utf8_percent_encode(s, SET).to_string()
}

/// Build the github.com release-download URL from a repo / tag / asset, or `None`
/// if the inputs aren't safe. `repo` accepts a pasted URL or `owner/name` and is
/// kept strict (it's the SSRF-sensitive path prefix); tag/asset may be richer
/// filenames and are percent-encoded.
fn github_asset_url(repo_in: &str, tag_in: &str, asset_in: &str) -> Option<String> {
    let repo = repo_in
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("github.com/")
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let tag = tag_in.trim();
    let asset = asset_in.trim();
    let repo_ok = repo.matches('/').count() == 1 && repo.split('/').all(safe_seg);
    if !repo_ok || !safe_path_seg(tag) || !safe_path_seg(asset) {
        return None;
    }
    Some(format!(
        "https://github.com/{repo}/releases/download/{}/{}",
        enc_seg(tag),
        enc_seg(asset)
    ))
}

// Fetch a GitHub release asset server-side and cache it by content hash, so a
// pack can pull a GitHub-only mod (open-smrt-network, hidemymods) as a normal
// smrt_cache source -- no new wire source type. The host is fixed to github.com,
// repo is kept strict (owner/name), and tag/asset are single, percent-encoded
// path segments (no separators, no traversal) -- not an open SSRF sink. See
// `github_asset_url`.
async fn ingest_github(
    State(state): State<AppState>,
    Json(req): Json<GithubIngest>,
) -> Result<(StatusCode, Json<PutCacheResponse>), ApiError> {
    let url = github_asset_url(&req.repo, &req.tag, &req.asset).ok_or_else(|| {
        ApiError::BadRequest(
            "repo must be owner/name (a github.com URL is ok); tag and asset must each be a single path segment".into(),
        )
    })?;
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

// ── config revisions (optimistic concurrency) ───────────────────────────────
//
// Config edits used to be a plain read-modify-write: two accounts editing one
// pack each saved against their own snapshot, and whoever saved second
// overwrote the first with no error and no trace (#52). A config now carries an
// `ETag` of its authored content, and a save may state which revision it edited
// via `If-Match`. A save whose base no longer matches disk is refused instead of
// applied, so the loser re-reads and reapplies rather than losing the work
// silently. The header is optional: a script or CLI that sends none writes
// unconditionally, exactly as before.

/// What a conditional write requires of the stored config: any config at all
/// (`If-Match: *`), or one specific revision.
enum Precondition {
    Any,
    Rev(String),
}

/// The `If-Match` precondition on this request, if it carries one. Entity tags
/// are opaque strings here, so the quoting and any weak-validator prefix are
/// stripped and what remains is compared verbatim. Only the single-tag form is
/// understood -- a tag list is not a shape this API's own clients produce, and
/// treating one as a mismatch refuses the write rather than waving it through.
fn precondition(headers: &HeaderMap) -> Option<Precondition> {
    let raw = headers.get(header::IF_MATCH)?.to_str().ok()?.trim();
    if raw == "*" {
        return Some(Precondition::Any);
    }
    let tag = raw
        .strip_prefix("W/")
        .unwrap_or(raw)
        .trim_matches('"')
        .to_string();
    (!tag.is_empty()).then_some(Precondition::Rev(tag))
}

/// Whether a conditional write may proceed against the revision now on disk
/// (`None` = the pack has no stored config).
fn check_precondition(
    pre: Option<&Precondition>,
    current: Option<&str>,
    pack_id: &str,
) -> Result<(), ApiError> {
    match (pre, current) {
        (None, _) => Ok(()),
        (Some(Precondition::Any), Some(_)) => Ok(()),
        (Some(Precondition::Rev(want)), Some(now)) if want == now => Ok(()),
        (Some(Precondition::Rev(want)), Some(now)) => Err(ApiError::Conflict(format!(
            "pack {pack_id:?} was saved by someone else since this edit began \
             (edited {}, now {}); reload the config and reapply the change",
            short_rev(want),
            short_rev(now)
        ))),
        (Some(_), None) => Err(ApiError::Conflict(format!(
            "pack {pack_id:?} has no stored config to match against; it was deleted or never existed"
        ))),
    }
}

/// A revision in an operator-facing message: enough to tell two apart, short
/// enough to read.
fn short_rev(rev: &str) -> &str {
    rev.get(..8).unwrap_or(rev)
}

/// The config's revision, or an internal error -- a config that cannot be
/// serialized has no revision to compare, and answering with one that always
/// matches would put the silent overwrite back.
fn rev_of(cfg: &PackConfig) -> Result<String, ApiError> {
    cfg.edit_rev()
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("config rev: {e}")))
}

/// The `ETag` response header carrying a config revision.
fn etag(rev: &str) -> [(HeaderName, String); 1] {
    [(header::ETAG, format!("\"{rev}\""))]
}

async fn get_pack_config(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
) -> Result<([(HeaderName, String); 1], Json<PackConfig>), ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    let cfg = state.storage.load_pack_config(&pack_id).await?;
    let rev = rev_of(&cfg)?;
    Ok((etag(&rev), Json(cfg)))
}

async fn put_pack_config(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
    headers: HeaderMap,
    Json(mut cfg): Json<PackConfig>,
) -> Result<(StatusCode, [(HeaderName, String); 1], Json<PackConfig>), ApiError> {
    // The path id is authoritative; reject a body that disagrees so a
    // mis-targeted PUT can't write one pack's config under another's id.
    if cfg.pack_id != pack_id {
        return Err(ApiError::BadRequest(format!(
            "body pack_id {:?} does not match path {:?}",
            cfg.pack_id, pack_id
        )));
    }
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    // One row per mod and per asset dest: the same artifact twice, or two rows
    // writing one path, is an authoring mistake no build can act on. Checked on
    // the incoming body, before anything server-side touches it.
    if let Some(dup) = cfg.duplicate_declaration() {
        return Err(ApiError::BadRequest(dup));
    }
    let pre = precondition(&headers);
    // From here to the write, this pack's config is ours alone: the stored
    // config is read, its server-controlled fields and pulled dependencies are
    // carried over, and the result is written. Two of those interleaving is the
    // same lost update `If-Match` refuses, arriving from the other side.
    let _guard = state.storage.lock_pack_config(&pack_id).await;
    // owner / tier / visibility / fork_of are server-controlled and never trusted
    // from the client. On an edit, carry them from the stored config so a member
    // can't reassign ownership, self-promote to official, or publish. On create,
    // derive them from the id's namespace: a community id (u/<uid>/...) is a draft
    // owned by that uid; a flat id is an official published operator pack.
    match state.storage.load_pack_config(&pack_id).await {
        Ok(existing) => {
            check_precondition(pre.as_ref(), Some(&rev_of(&existing)?), &pack_id)?;
            cfg.owner = existing.owner;
            cfg.tier = existing.tier;
            cfg.visibility = existing.visibility;
            cfg.fork_of = existing.fork_of.clone();
            // Sticky pulled dependencies: entries the fill appended on earlier
            // saves survive a body that lacks them (the client may never have
            // seen them), independent of whether the fill below can reach
            // Modrinth right now. Orphans are pruned by the fill itself once
            // nothing declared reaches them.
            crate::authoring::depfill::merge_pulled(&existing, &mut cfg);
        }
        Err(_) => {
            check_precondition(pre.as_ref(), None, &pack_id)?;
            match super::auth::pack_namespace_uid(&pack_id) {
                Some(uid) => {
                    super::auth::require_terms(&state, identity.uid).await?;
                    cfg.owner = uid;
                    cfg.tier = PackTier::Community;
                    cfg.visibility = Visibility::Draft;
                    cfg.fork_of = None;
                }
                None => {
                    cfg.owner = identity.uid;
                    cfg.tier = PackTier::Official;
                    cfg.visibility = Visibility::Published;
                    cfg.fork_of = None;
                }
            }
        }
    }
    // Pull in each mod's missing hard dependencies (Modrinth first, the mirror's
    // own cache second) and record the resolved requires graph, so the operator
    // never hand-manages libraries and the build can derive required-ness.
    // Best-effort: a Modrinth outage must not block saving a config, so a fill
    // error is logged and the raw config is saved.
    let cached: std::collections::HashSet<String> = state
        .storage
        .list_cache_inventory()
        .await
        .map(|inv| inv.into_iter().map(|e| e.sha1).collect())
        .unwrap_or_default();
    if let Err(e) = crate::authoring::depfill::fill_dependencies(
        &mut cfg,
        &state.registry,
        &state.modrinth,
        &cached,
    )
    .await
    {
        tracing::warn!(pack_id = %pack_id, error = %e, "dependency auto-fill failed; saving config as-is");
    }
    state.storage.save_pack_config(&pack_id, &cfg).await?;
    // the revision of what was just stored, so the client that saved it edits
    // on from there without a re-read
    let rev = rev_of(&cfg)?;
    audit(
        &state,
        &identity,
        "pack.config",
        Some(&pack_id),
        Some(&format!("{} mods", cfg.mods.len())),
    )
    .await;
    Ok((StatusCode::CREATED, etag(&rev), Json(cfg)))
}

#[derive(serde::Deserialize)]
struct RevertParams {
    version: String,
}

// Overwrite the authoring config with one reconstructed from a published build's
// manifest + summary -- the panel's "revert to build" affordance, since config
// edits autosave with no history of their own. Returns the new config so the
// editor can swap to it without a reload.
async fn revert_pack_config(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
    Query(p): Query<RevertParams>,
) -> Result<([(HeaderName, String); 1], Json<PackConfig>), ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    let manifest = state
        .storage
        .load_manifest_version(&pack_id, &p.version)
        .await?;
    let summary = state.storage.load_pack_summary(&pack_id).await?;
    let cfg = reconstruct_config(&manifest, &summary);
    // A revert is a deliberate overwrite of whatever is there, so it states no
    // precondition -- but it still takes the lock, so it cannot land in the
    // middle of a save's read-modify-write, and it answers with the revision it
    // produced so the editor keeps saving conditionally afterwards.
    let _guard = state.storage.lock_pack_config(&pack_id).await;
    state.storage.save_pack_config(&pack_id, &cfg).await?;
    let rev = rev_of(&cfg)?;
    audit(
        &state,
        &identity,
        "pack.revert",
        Some(&pack_id),
        Some(&p.version),
    )
    .await;
    Ok((etag(&rev), Json(cfg)))
}

#[derive(serde::Deserialize)]
struct DuplicatePackReq {
    target_id: String,
    // Optional loader override, so a loader variant (e.g. a Cleanroom trial of a
    // Forge pack) is one call. Absent -> keep the source loader.
    #[serde(default)]
    loader: Option<LoaderSpec>,
}

// Clone a curated pack under a new id -- copy config + the per-pack static tree,
// with an optional loader override -- leaving the source untouched. The shared
// content-addressed cache means mod sources resolve without re-upload; the
// operator builds the new pack via the usual build endpoint. This is the
// single-admin primitive; ownership-aware forks (#36) layer on top of it.
async fn duplicate_pack(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
    Json(req): Json<DuplicatePackReq>,
) -> Result<(StatusCode, Json<PackConfig>), ApiError> {
    // must own (or admin) both the source pack and the target namespace
    if !super::auth::may_author(&identity, &pack_id)
        || !super::auth::may_author(&identity, &req.target_id)
    {
        return Err(ApiError::Forbidden);
    }
    // the clone is owned by the target's namespace (a member's own uid), or by
    // the caller for an official (flat) target
    let owner = super::auth::pack_namespace_uid(&req.target_id).unwrap_or(identity.uid);
    let cfg = state
        .storage
        .duplicate_pack(&pack_id, &req.target_id, req.loader, owner, None)
        .await?;
    Ok((StatusCode::CREATED, Json(cfg)))
}

/// Every pack summary, unfiltered -- the operator's view, including drafts,
/// unlisted, and community packs that the public `/v1/packs` listing hides.
async fn list_all_pack_summaries(
    State(state): State<AppState>,
) -> Result<Json<Vec<PackSummary>>, ApiError> {
    Ok(Json(state.storage.list_pack_summaries().await?))
}

#[derive(serde::Deserialize)]
struct VisibilityReq {
    visibility: Visibility,
}

/// Delete a pack and everything under it. Ownership-gated like the rest of
/// authoring: a member deletes only their own community packs.
async fn delete_pack(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    state.storage.delete_pack(&pack_id).await?;
    audit(&state, &identity, "pack.delete", Some(&pack_id), None).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Publish / unpublish (or unlist) a pack. Takes effect on the public listing
/// immediately (see `Storage::set_pack_visibility`).
async fn set_pack_visibility(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
    Json(req): Json<VisibilityReq>,
) -> Result<StatusCode, ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    state
        .storage
        .set_pack_visibility(&pack_id, req.visibility)
        .await?;
    audit(
        &state,
        &identity,
        "pack.visibility",
        Some(&pack_id),
        Some(&format!("{:?}", req.visibility)),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
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

#[cfg(test)]
mod tests {
    use super::{Precondition, check_precondition, github_asset_url, precondition};
    use axum::http::{HeaderMap, HeaderValue, header};

    fn headers(if_match: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::IF_MATCH, HeaderValue::from_str(if_match).unwrap());
        h
    }

    #[test]
    fn if_match_reads_quoted_weak_and_wildcard_tags() {
        // what the panel echoes back from an ETag, and what a hand-written
        // request is likely to carry, must mean the same revision
        for raw in ["\"abc123\"", "abc123", "W/\"abc123\"", "  \"abc123\" "] {
            match precondition(&headers(raw)) {
                Some(Precondition::Rev(rev)) => assert_eq!(rev, "abc123", "raw {raw:?}"),
                _ => panic!("expected a revision for {raw:?}"),
            }
        }
        assert!(matches!(
            precondition(&headers("*")),
            Some(Precondition::Any)
        ));
        assert!(precondition(&HeaderMap::new()).is_none(), "header absent");
        assert!(precondition(&headers("\"\"")).is_none(), "empty tag");
    }

    #[test]
    fn a_stale_base_is_refused_and_a_current_one_passes() {
        let stale = Precondition::Rev("aaaa1111".into());
        let current = Precondition::Rev("bbbb2222".into());

        assert!(check_precondition(None, Some("bbbb2222"), "Industrial").is_ok());
        assert!(check_precondition(Some(&current), Some("bbbb2222"), "Industrial").is_ok());
        assert!(
            check_precondition(Some(&Precondition::Any), Some("bbbb2222"), "Industrial").is_ok()
        );

        let err = check_precondition(Some(&stale), Some("bbbb2222"), "Industrial")
            .expect_err("a base that no longer matches disk is a conflict");
        let msg = err.to_string();
        assert!(msg.contains("Industrial"), "names the pack: {msg}");
        assert!(
            msg.contains("aaaa1111") && msg.contains("bbbb2222"),
            "{msg}"
        );

        // a config that is gone answers a conditional write with a conflict,
        // not a silent create over the top of a delete
        assert!(check_precondition(Some(&stale), None, "Industrial").is_err());
        assert!(check_precondition(Some(&Precondition::Any), None, "Industrial").is_err());
        assert!(
            check_precondition(None, None, "Industrial").is_ok(),
            "an unconditional first save still creates"
        );
    }

    #[test]
    fn github_url_accepts_plain_repo_and_simple_names() {
        assert_eq!(
            github_asset_url("Kitty-Hivens/open-smrt-network", "v1.2.3", "osn-1.12.2.jar"),
            Some(
                "https://github.com/Kitty-Hivens/open-smrt-network/releases/download/v1.2.3/osn-1.12.2.jar"
                    .into()
            )
        );
    }

    #[test]
    fn github_url_normalizes_a_pasted_url() {
        // a pasted browser URL (scheme + host, trailing .git/slash) still resolves
        for repo in [
            "https://github.com/owner/repo",
            "github.com/owner/repo/",
            "owner/repo.git",
        ] {
            assert_eq!(
                github_asset_url(repo, "v1", "a.jar"),
                Some("https://github.com/owner/repo/releases/download/v1/a.jar".into()),
                "repo {repo:?}"
            );
        }
    }

    #[test]
    fn github_url_percent_encodes_rich_asset_names() {
        // spaces / parens / plus -- common in real release assets -- are encoded,
        // not rejected
        let url = github_asset_url("o/r", "1.0+build5", "Cool Mod (1.12.2).jar").unwrap();
        assert_eq!(
            url,
            "https://github.com/o/r/releases/download/1.0%2Bbuild5/Cool%20Mod%20(1.12.2).jar"
        );
    }

    #[test]
    fn github_url_rejects_unsafe_inputs() {
        // repo must be exactly owner/name
        assert!(github_asset_url("owner/repo/extra", "v1", "a.jar").is_none());
        assert!(github_asset_url("justowner", "v1", "a.jar").is_none());
        // tag/asset can't add path depth or traverse
        assert!(github_asset_url("o/r", "v1/x", "a.jar").is_none());
        assert!(github_asset_url("o/r", "v1", "sub/a.jar").is_none());
        assert!(github_asset_url("o/r", "..", "a.jar").is_none());
        assert!(github_asset_url("o/r", "v1", "").is_none());
    }
}
