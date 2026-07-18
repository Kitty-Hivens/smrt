use super::ApiError;
use crate::authoring::jar_icon;
use crate::domain::*;
use crate::registry::model::ModDetail;
use crate::registry::queries;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use std::collections::{HashMap, HashSet};
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
        .route(
            "/v1/packs/:pack_id/manifest/versions",
            get(list_manifest_versions),
        )
        .route(
            "/v1/packs/:pack_id/manifest/:version",
            get(get_manifest_version),
        )
        .route("/v1/packs/:pack_id/static/*rel_path", get(get_pack_static))
        .route("/v1/servers", get(list_servers))
        .route("/v1/servers/:server_id", get(get_server))
        .route("/v1/featured", get(get_featured))
        .route("/v1/cache/:prefix/:filename", get(get_cache_jar))
        .route("/v1/cache/icon/:sha1", get(get_cache_icon))
        .route("/v1/cache/inventory", get(get_cache_inventory))
        .route("/v1/community", get(list_community))
        .route("/v1/mods/:key", get(get_mod_detail))
        .route("/v1/users/:uid/avatar", get(get_user_avatar))
        .with_state(state)
}

// ── /v1/mods/:id ─────────────────────────────────────────────────────────────

/// The public read model behind a single mod's page: identity, releases (files),
/// the relations that touch it, and the packs that ship it. Read-only and
/// unauthenticated -- mod metadata is not sensitive, and the mirror already
/// serves the jars themselves. `used_by` is narrowed to official + published
/// packs so a guest cannot learn a draft's name from it; file `cached` flags are
/// set here against the live cache. Operators reuse this view and reach the gated
/// edit/diff endpoints separately.
///
/// `key` is either a numeric mod id (the graph and registry navigate by id) or a
/// `sha1:<hash>` artifact reference (a pack's mod list has the jar's sha1, not the
/// mod id) -- both resolve to the same page.
#[utoipa::path(
    get,
    path = "/v1/mods/{key}",
    tag = "public",
    params(("key" = String, Path,
        description = "Numeric mod id, or `sha1:<hash>` of any of the mod's artifacts")),
    responses(
        (status = 200, description = "The mod's public page model", body = ModDetail),
        (status = 404, description = "No mod resolves from the key")
    )
)]
pub(crate) async fn get_mod_detail(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<ModDetail>, ApiError> {
    let reg = state.registry.clone();
    let detail = tokio::task::spawn_blocking(move || {
        reg.with_conn(|c| {
            let mod_id = if let Ok(id) = key.parse::<i64>() {
                Some(id)
            } else if let Some(sha1) = key.strip_prefix("sha1:") {
                queries::mod_id_for_sha1(c, sha1)?
            } else {
                None
            };
            match mod_id {
                Some(id) => queries::mod_detail(c, id),
                None => Ok(None),
            }
        })
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("mod detail task: {e}")))??;
    let Some(mut detail) = detail else {
        return Err(ApiError::NotFound);
    };

    let cached: HashSet<String> = state
        .storage
        .list_cache_inventory()
        .await?
        .into_iter()
        .map(|e| e.sha1)
        .collect();
    for rel in &mut detail.releases {
        for f in &mut rel.files {
            f.cached = cached.contains(&f.sha1);
        }
    }

    // never surface a draft/unlisted/community pack name to a guest through the
    // "used by" list -- keep it to the same set the launcher's catalog exposes.
    let public_packs: HashSet<String> = state
        .storage
        .list_pack_summaries()
        .await?
        .into_iter()
        .filter(|p| p.tier == PackTier::Official && p.visibility == Visibility::Published)
        .map(|p| p.pack_id)
        .collect();
    detail.used_by.retain(|u| public_packs.contains(&u.pack_id));

    Ok(Json(detail))
}

// ── /v1/health ─────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/health",
    tag = "public",
    responses((status = 200, description = "Mirror is up", body = Health))
)]
pub(crate) async fn health() -> Json<Health> {
    Json(Health {
        schema_version: SCHEMA_VERSION,
        status: "ok",
        // crate version + git short sha, stamped by build.rs -- moves with the code
        version: env!("SMRT_BUILD_VERSION"),
    })
}

// ── /v1/packs ──────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/packs",
    tag = "public",
    responses((status = 200, description = "The launcher catalog: official, published packs", body = PackListing))
)]
pub(crate) async fn list_packs(
    State(state): State<AppState>,
) -> Result<Json<PackListing>, ApiError> {
    // The launcher's catalog is official + published only; drafts, unlisted, and
    // community packs are reached through other surfaces, never this listing.
    let mut packs: Vec<PackSummary> = state
        .storage
        .list_pack_summaries()
        .await?
        .into_iter()
        .filter(|p| p.tier == PackTier::Official && p.visibility == Visibility::Published)
        .collect();
    for p in &mut packs {
        enrich_latest_build(&state, p).await;
    }
    Ok(Json(PackListing {
        schema_version: SCHEMA_VERSION,
        generated_at: now_rfc3339(),
        packs,
    }))
}

/// Stamp `latest_built_at` / `latest_channel` onto a summary from the latest
/// manifest's header. Read-time derivation, so the values can never drift from
/// the manifest they describe; a pack without a readable build simply keeps
/// both fields absent.
async fn enrich_latest_build(state: &AppState, summary: &mut PackSummary) {
    if let Ok(Some(info)) = state.storage.latest_build_info(&summary.pack_id).await {
        summary.latest_built_at = Some(info.built_at);
        summary.latest_channel = Some(info.channel);
    }
}

/// Published community packs for the site's Community view -- browseable here but
/// never part of the launcher's official `/v1/packs` catalog. Each carries the
/// owner's login (resolved from the uid) for the byline.
#[utoipa::path(
    get,
    path = "/v1/community",
    tag = "public",
    responses((status = 200, description = "Published community packs with owner byline", body = Vec<CommunityPack>))
)]
pub(crate) async fn list_community(
    State(state): State<AppState>,
) -> Result<Json<Vec<CommunityPack>>, ApiError> {
    let summaries: Vec<PackSummary> = state
        .storage
        .list_pack_summaries()
        .await?
        .into_iter()
        .filter(|p| p.tier == PackTier::Community && p.visibility == Visibility::Published)
        .collect();

    let acc = state.accounts.clone();
    let users = tokio::task::spawn_blocking(move || acc.list_users())
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("community users task: {e}")))??;
    let logins: HashMap<i64, String> = users.into_iter().map(|u| (u.github_uid, u.login)).collect();

    let mut out: Vec<CommunityPack> = summaries
        .into_iter()
        .map(|s| {
            let owner_login = logins
                .get(&s.owner)
                .cloned()
                .unwrap_or_else(|| format!("uid {}", s.owner));
            CommunityPack {
                summary: s,
                owner_login,
            }
        })
        .collect();
    for p in &mut out {
        enrich_latest_build(&state, &mut p.summary).await;
    }
    Ok(Json(out))
}

#[utoipa::path(
    get,
    path = "/v1/packs/{pack_id}",
    tag = "public",
    params(("pack_id" = String, Path, description = "Pack identifier")),
    responses((status = 200, body = PackSummary), (status = 404, description = "No such pack"))
)]
pub(crate) async fn get_pack_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pack_id): Path<String>,
) -> Result<Json<PackSummary>, ApiError> {
    let mut summary = state.storage.load_pack_summary(&pack_id).await?;
    gate_summary(&state, &headers, &pack_id, &summary).await?;
    enrich_latest_build(&state, &mut summary).await;
    Ok(Json(summary))
}

#[utoipa::path(
    get,
    path = "/v1/packs/{pack_id}/manifest",
    tag = "public",
    params(("pack_id" = String, Path, description = "Pack identifier")),
    responses(
        (status = 200, description = "The pack's latest manifest", body = PackManifest),
        (status = 404, description = "No such pack, or no build yet")
    )
)]
pub(crate) async fn get_latest_manifest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pack_id): Path<String>,
) -> Result<Json<PackManifest>, ApiError> {
    gate_pack_read(&state, &headers, &pack_id).await?;
    Ok(Json(state.storage.load_latest_manifest(&pack_id).await?))
}

#[utoipa::path(
    get,
    path = "/v1/packs/{pack_id}/manifest/{version}",
    tag = "public",
    params(
        ("pack_id" = String, Path, description = "Pack identifier"),
        ("version" = String, Path, description = "Exact pack version label")
    ),
    responses(
        (status = 200, description = "The manifest of that build", body = PackManifest),
        (status = 404, description = "No such pack or version")
    )
)]
pub(crate) async fn get_manifest_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pack_id, version)): Path<(String, String)>,
) -> Result<Json<PackManifest>, ApiError> {
    gate_pack_read(&state, &headers, &pack_id).await?;
    Ok(Json(
        state
            .storage
            .load_manifest_version(&pack_id, &version)
            .await?,
    ))
}

#[utoipa::path(
    get,
    path = "/v1/packs/{pack_id}/manifest/versions",
    tag = "public",
    params(("pack_id" = String, Path, description = "Pack identifier")),
    responses(
        (status = 200,
         description = "Every retained build: bare labels (oldest first) plus \
                        per-build metadata (newest first) and the current latest",
         body = ManifestVersionsListing),
        (status = 404, description = "No such pack")
    )
)]
pub(crate) async fn list_manifest_versions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pack_id): Path<String>,
) -> Result<Json<ManifestVersionsListing>, ApiError> {
    gate_pack_read(&state, &headers, &pack_id).await?;
    let builds = state.storage.list_manifest_builds(&pack_id).await?;
    let latest = state.storage.latest_manifest_version(&pack_id).await?;
    // One directory scan feeds both shapes: `builds` is newest-first for
    // version pickers, `versions` keeps the original oldest-first label list
    // for clients that predate the rich form.
    let versions = builds.iter().rev().map(|b| b.version.clone()).collect();
    Ok(Json(ManifestVersionsListing {
        schema_version: SCHEMA_VERSION,
        pack_id,
        latest,
        versions,
        builds,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/packs/{pack_id}/static/{rel_path}",
    tag = "public",
    params(
        ("pack_id" = String, Path, description = "Pack identifier"),
        ("rel_path" = String, Path, description = "Path under the pack's static tree")
    ),
    responses(
        (status = 200, description = "The static file bytes, content type by extension"),
        (status = 404, description = "No such pack or file")
    )
)]
pub(crate) async fn get_pack_static(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pack_id, rel_path)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    gate_pack_read(&state, &headers, &pack_id).await?;
    let path = state.storage.pack_static_path(&pack_id, &rel_path)?;
    if tokio::fs::metadata(&path).await.is_err() {
        return Err(ApiError::NotFound);
    }
    serve_file(&path, content_type_for(&rel_path)).await
}

/// Draft packs are private -- readable only by their owner (or an admin), so a
/// work-in-progress pack does not leak by direct id. Unlisted and published stay
/// readable by id (unlisted is off-catalog but link-shareable). A denied draft
/// answers `NotFound`, not 403, so a private pack's existence stays unconfirmed.
async fn gate_summary(
    state: &AppState,
    headers: &HeaderMap,
    pack_id: &str,
    summary: &PackSummary,
) -> Result<(), ApiError> {
    if summary.visibility != Visibility::Draft {
        return Ok(());
    }
    match super::auth::optional_identity(state, headers).await {
        Some(id) if super::auth::may_author(&id, pack_id) => Ok(()),
        _ => Err(ApiError::NotFound),
    }
}

async fn gate_pack_read(
    state: &AppState,
    headers: &HeaderMap,
    pack_id: &str,
) -> Result<(), ApiError> {
    let summary = state.storage.load_pack_summary(pack_id).await?;
    gate_summary(state, headers, pack_id, &summary).await
}

// ── /v1/servers ────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/servers",
    tag = "public",
    responses((status = 200, description = "Curated server listing", body = ServerListing))
)]
pub(crate) async fn list_servers(
    State(state): State<AppState>,
) -> Result<Json<ServerListing>, ApiError> {
    let servers = state.storage.list_servers().await?;
    Ok(Json(ServerListing {
        schema_version: SCHEMA_VERSION,
        generated_at: now_rfc3339(),
        servers,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/servers/{server_id}",
    tag = "public",
    params(("server_id" = String, Path, description = "Server identifier")),
    responses(
        (status = 200, body = ServerEntry),
        (status = 404, description = "No such server")
    )
)]
pub(crate) async fn get_server(
    State(state): State<AppState>,
    Path(server_id): Path<String>,
) -> Result<Json<ServerEntry>, ApiError> {
    Ok(Json(state.storage.load_server(&server_id).await?))
}

// ── /v1/featured ───────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/featured",
    tag = "public",
    responses((status = 200, description = "Featured packs and servers", body = Featured))
)]
pub(crate) async fn get_featured(
    State(state): State<AppState>,
) -> Result<Json<Featured>, ApiError> {
    Ok(Json(state.storage.load_featured().await?))
}

// ── /v1/cache ──────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/v1/cache/{prefix}/{filename}",
    tag = "public",
    params(
        ("prefix" = String, Path, description = "First two hex chars of the sha1"),
        ("filename" = String, Path, description = "`<sha1>.jar`")
    ),
    responses(
        (status = 200, description = "The cached jar bytes"),
        (status = 404, description = "Not cached, or taken down")
    )
)]
pub(crate) async fn get_cache_jar(
    State(state): State<AppState>,
    Path((prefix, filename)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let sha1 = filename
        .strip_suffix(".jar")
        .ok_or_else(|| ApiError::BadRequest("cache path must end in .jar".into()))?;
    // A taken-down jar must not be served even if its bytes are still on disk.
    if state.storage.is_sha1_removed(sha1).await? {
        return Err(ApiError::NotFound);
    }
    let path = state.storage.cache_jar_path(&prefix, sha1)?;
    if tokio::fs::metadata(&path).await.is_err() {
        return Err(ApiError::NotFound);
    }
    serve_file(&path, "application/java-archive").await
}

// A cached mod's own embedded icon (mcmod.info logoFile / pack.png / fabric icon),
// so any browser of the registry -- guest, member, operator -- sees the real icon
// a self-hosted jar carries. Public alongside the jar itself: an icon extracted
// from a jar anyone can download is no more sensitive than the download. Immutable
// per sha1, so it caches hard; 404 when the jar has no icon (the caller falls back
// to a letter avatar).
#[utoipa::path(
    get,
    path = "/v1/cache/icon/{sha1}",
    tag = "public",
    params(("sha1" = String, Path, description = "Full 40-char sha1 of a cached jar")),
    responses(
        (status = 200, description = "The jar's embedded icon; immutable-cacheable"),
        (status = 404, description = "Jar not cached, or carries no icon")
    )
)]
pub(crate) async fn get_cache_icon(
    State(state): State<AppState>,
    Path(sha1): Path<String>,
) -> Result<Response, ApiError> {
    if sha1.len() != 40 || !sha1.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest("sha1 must be 40 hex chars".into()));
    }
    let path = state.storage.cache_jar_path(&sha1[..2], &sha1)?;
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let icon = tokio::task::spawn_blocking(move || jar_icon(&bytes))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("icon extract task: {e}")))??;
    let (img, content_type) = icon.ok_or(ApiError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            // bytes come from an untrusted jar; pin the type so the browser can't
            // sniff a "pack.png" that actually contains markup into something active
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        img,
    )
        .into_response())
}

#[utoipa::path(
    get,
    path = "/v1/cache/inventory",
    tag = "public",
    responses((status = 200, description = "Every cached artifact (sha1, size)", body = CacheInventory))
)]
pub(crate) async fn get_cache_inventory(
    State(state): State<AppState>,
) -> Result<Json<CacheInventory>, ApiError> {
    let entries = state.storage.list_cache_inventory().await?;
    Ok(Json(CacheInventory {
        schema_version: SCHEMA_VERSION,
        generated_at: now_rfc3339(),
        entries,
    }))
}

// ── /v1/users/:uid/avatar ──────────────────────────────────────────────────

/// Proxy a GitHub avatar through the mirror, keyed by the numeric uid we already
/// store. Serving it from our own origin means the panel never hotlinks
/// `avatars.githubusercontent.com` from the viewer's browser -- no viewer IP
/// handed to GitHub, no third-party origin on the page. A bad uid or an upstream
/// miss is a 404 the panel falls back from to a letter tile.
#[utoipa::path(
    get,
    path = "/v1/users/{uid}/avatar",
    tag = "public",
    params(("uid" = i64, Path, description = "GitHub numeric uid")),
    responses(
        (status = 200, description = "The avatar image, proxied through the mirror"),
        (status = 404, description = "Bad uid or upstream miss")
    )
)]
pub(crate) async fn get_user_avatar(
    State(state): State<AppState>,
    Path(uid): Path<i64>,
) -> Result<Response, ApiError> {
    if uid <= 0 {
        return Err(ApiError::NotFound);
    }
    let url = format!("https://avatars.githubusercontent.com/u/{uid}?s=160&v=4");
    let (bytes, content_type) = state
        .modrinth
        .fetch_image(&url)
        .await
        .map_err(|_| ApiError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=86400".to_string()),
            // proxied third-party bytes: pin the type so the browser can't sniff
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ],
        bytes,
    )
        .into_response())
}

// ── helpers ────────────────────────────────────────────────────────────────

async fn serve_file(path: &std::path::Path, content_type: &str) -> Result<Response, ApiError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let meta = file
        .metadata()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
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

fn content_type_for(rel_path: &str) -> &'static str {
    let lower = rel_path.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "json" => "application/json",
        "toml" => "application/toml",
        "txt" | "cfg" | "properties" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn now_rfc3339() -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}
