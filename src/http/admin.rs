use super::ApiError;
use crate::authoring::{ValidateReport, jar_icon, modrinth, reconstruct_config, validate};
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
        .route("/v1/admin/cache/icon/:sha1", get(get_cache_icon))
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
            "/v1/admin/packs/:pack_id/config/revert",
            post(revert_pack_config),
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

// Serve a cached mod's own embedded icon (mcmod.info logoFile / pack.png /
// fabric icon), so the panel can show real mod icons for self-hosted jars. The
// content is immutable per sha1, so it caches hard in the browser. 404 when the
// jar carries no icon -- the panel falls back to a letter avatar.
async fn get_cache_icon(
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
    Path(pack_id): Path<String>,
    Query(p): Query<RevertParams>,
) -> Result<Json<PackConfig>, ApiError> {
    let manifest = state
        .storage
        .load_manifest_version(&pack_id, &p.version)
        .await?;
    let summary = state.storage.load_pack_summary(&pack_id).await?;
    let cfg = reconstruct_config(&manifest, &summary);
    state.storage.save_pack_config(&pack_id, &cfg).await?;
    Ok(Json(cfg))
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
    use super::github_asset_url;

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
