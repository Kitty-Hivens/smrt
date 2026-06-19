//! Admin registry endpoints: trigger a harvest and read the mod-identity index
//! (which packs use a mod, all versions, orphans, loader eligibility). Auth-
//! gated like the rest of `/v1/admin` -- these expose pack composition.
//!
//! Phase 1 runs the harvest synchronously and returns the report; a single
//! operator tolerates the few-second wait. Streaming it as a job is a Phase 2
//! nicety, not needed here.

use super::ApiError;
use crate::authoring::harvest::{self, HarvestReport};
use crate::registry::model::{
    BuildModRow, BuildSummary, EligibleArtifact, ModSummary, ModUse, OrphanJar, RegistryStats,
    VersionRow,
};
use crate::registry::queries;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/admin/registry/harvest", post(post_harvest))
        .route("/v1/admin/registry/stats", get(get_stats))
        .route("/v1/admin/registry/orphans", get(get_orphans))
        .route("/v1/admin/registry/eligible", get(get_eligible))
        // registry browser: mods (faceted list), versions by surrogate id, builds
        .route("/v1/admin/registry/mods", get(get_mods))
        .route("/v1/admin/registry/mod-versions/:mod_id", get(get_versions_by_id))
        .route("/v1/admin/registry/builds", get(get_builds))
        .route(
            "/v1/admin/registry/builds/:pack_id/:pack_version",
            get(get_build_mods),
        )
        .route(
            "/v1/admin/registry/mods/:alias_source/:external_key",
            get(get_mod_versions),
        )
        .route(
            "/v1/admin/registry/mods/:alias_source/:external_key/uses",
            get(get_mod_uses),
        )
        // authored moderation (Phase 2)
        .route(
            "/v1/admin/registry/packs/:pack_id/provenance",
            put(put_provenance),
        )
        .route("/v1/admin/registry/conflicts", post(post_conflict))
        .route("/v1/admin/registry/backup", post(post_backup))
        .layer(from_fn_with_state(state.clone(), super::auth::require_auth))
        .with_state(state)
}

/// Run a blocking registry write off the async runtime.
async fn run_write<T>(
    state: &AppState,
    f: impl FnOnce(&crate::registry::Registry) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
{
    let reg = state.registry.clone();
    let res = tokio::task::spawn_blocking(move || f(&reg))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("registry write task: {e}")))?;
    Ok(res?)
}

/// Run a registry read off the async runtime (rusqlite is blocking).
async fn run_query<T, F>(state: &AppState, f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> anyhow::Result<T> + Send + 'static,
{
    let reg = state.registry.clone();
    let res = tokio::task::spawn_blocking(move || reg.with_conn(f))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("registry query task: {e}")))?;
    Ok(res?)
}

/// The sha1s the mirror actually holds in its local cache. The registry indexes
/// manifest-only (e.g. Modrinth-sourced) artifacts too, so "in the registry"
/// does not mean "on disk" -- the panel needs this to pick a `smrt_cache` vs a
/// `modrinth` source per artifact.
async fn cache_shas(state: &AppState) -> Result<std::collections::HashSet<String>, ApiError> {
    let inv = state.storage.list_cache_inventory().await?;
    Ok(inv.into_iter().map(|e| e.sha1).collect())
}

/// Force an immediate harvest and return the report. Runs the harvest directly
/// (not via the background scheduler) so the operator gets the counts back
/// synchronously. Overlap with a concurrent auto-harvest is safe: the writes are
/// idempotent and serialized by the registry's connection mutex, so the two just
/// converge on the same state.
async fn post_harvest(State(state): State<AppState>) -> Result<Json<HarvestReport>, ApiError> {
    let report =
        harvest::run_harvest(&state.storage, &state.modrinth, state.registry.clone()).await?;
    Ok(Json(report))
}

async fn get_stats(State(state): State<AppState>) -> Result<Json<RegistryStats>, ApiError> {
    Ok(Json(run_query(&state, queries::stats).await?))
}

async fn get_orphans(State(state): State<AppState>) -> Result<Json<Vec<OrphanJar>>, ApiError> {
    Ok(Json(run_query(&state, queries::orphan_jars).await?))
}

#[derive(Deserialize)]
struct EligibleQuery {
    loader: String,
}

async fn get_eligible(
    State(state): State<AppState>,
    Query(q): Query<EligibleQuery>,
) -> Result<Json<Vec<EligibleArtifact>>, ApiError> {
    Ok(Json(
        run_query(&state, move |c| queries::eligible_for_loader(c, &q.loader)).await?,
    ))
}

#[derive(Deserialize)]
struct ModsQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    loader: Option<String>,
    #[serde(default)]
    mc: Option<String>,
}

async fn get_mods(
    State(state): State<AppState>,
    Query(q): Query<ModsQuery>,
) -> Result<Json<Vec<ModSummary>>, ApiError> {
    // empty query params arrive as Some("") -- treat blank as "no filter"
    let blank = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let (q_, loader_, mc_) = (blank(&q.q), blank(&q.loader), blank(&q.mc));
    Ok(Json(
        run_query(&state, move |c| {
            queries::list_mods(c, q_.as_deref(), loader_.as_deref(), mc_.as_deref())
        })
        .await?,
    ))
}

async fn get_versions_by_id(
    State(state): State<AppState>,
    Path(mod_id): Path<i64>,
) -> Result<Json<Vec<VersionRow>>, ApiError> {
    let mut rows = run_query(&state, move |c| queries::versions_of_mod_by_id(c, mod_id)).await?;
    let cached = cache_shas(&state).await?;
    for r in &mut rows {
        r.cached = cached.contains(&r.sha1);
    }
    Ok(Json(rows))
}

async fn get_builds(State(state): State<AppState>) -> Result<Json<Vec<BuildSummary>>, ApiError> {
    Ok(Json(run_query(&state, queries::list_builds).await?))
}

async fn get_build_mods(
    State(state): State<AppState>,
    Path((pack_id, pack_version)): Path<(String, String)>,
) -> Result<Json<Vec<BuildModRow>>, ApiError> {
    let mut rows = run_query(&state, move |c| {
        queries::build_mods(c, &pack_id, &pack_version)
    })
    .await?;
    let cached = cache_shas(&state).await?;
    for r in &mut rows {
        r.cached = cached.contains(&r.sha1);
    }
    Ok(Json(rows))
}

async fn get_mod_versions(
    State(state): State<AppState>,
    Path((alias_source, external_key)): Path<(String, String)>,
) -> Result<Json<Vec<VersionRow>>, ApiError> {
    let mut rows = run_query(&state, move |c| {
        queries::versions_of_mod(c, &alias_source, &external_key)
    })
    .await?;
    let cached = cache_shas(&state).await?;
    for r in &mut rows {
        r.cached = cached.contains(&r.sha1);
    }
    Ok(Json(rows))
}

async fn get_mod_uses(
    State(state): State<AppState>,
    Path((alias_source, external_key)): Path<(String, String)>,
) -> Result<Json<Vec<ModUse>>, ApiError> {
    Ok(Json(
        run_query(&state, move |c| {
            queries::packs_using_mod(c, &alias_source, &external_key)
        })
        .await?,
    ))
}

// ── authored moderation (Phase 2) ────────────────────────────────────────────

#[derive(Deserialize)]
struct ProvenanceBody {
    provenance: String,
}

async fn put_provenance(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Json(body): Json<ProvenanceBody>,
) -> Result<StatusCode, ApiError> {
    if body.provenance != "sc" && body.provenance != "hivens" {
        return Err(ApiError::BadRequest(
            "provenance must be 'sc' or 'hivens'".into(),
        ));
    }
    run_write(&state, move |reg| {
        reg.set_provenance(&pack_id, &body.provenance)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ConflictBody {
    a_modid: String,
    b_modid: String,
    #[serde(default)]
    remove: bool,
}

async fn post_conflict(
    State(state): State<AppState>,
    Json(b): Json<ConflictBody>,
) -> Result<StatusCode, ApiError> {
    run_write(&state, move |reg| {
        reg.set_conflict(&b.a_modid, &b.b_modid, b.remove)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct BackupResult {
    path: String,
}

async fn post_backup(State(state): State<AppState>) -> Result<Json<BackupResult>, ApiError> {
    let dir = state.config.storage_dir.join("backups");
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::Internal(e.into()))?;
    let stamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let dest = dir.join(format!("registry-{stamp}.db"));
    let target = dest.clone();
    run_write(&state, move |reg| reg.backup_into(&target)).await?;
    Ok(Json(BackupResult {
        path: dest.to_string_lossy().into_owned(),
    }))
}
