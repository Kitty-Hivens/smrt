//! Admin registry endpoints: trigger a harvest and read the mod-identity index
//! (which packs use a mod, all versions, orphans, loader eligibility). Auth-
//! gated like the rest of `/v1/admin` -- these expose pack composition.
//!
//! Phase 1 runs the harvest synchronously and returns the report; a single
//! operator tolerates the few-second wait. Streaming it as a job is a Phase 2
//! nicety, not needed here.

use super::ApiError;
use crate::authoring::harvest::{self, HarvestReport};
use crate::registry::model::{EligibleArtifact, ModUse, OrphanJar, RegistryStats, VersionRow};
use crate::registry::queries;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/admin/registry/harvest", post(post_harvest))
        .route("/v1/admin/registry/stats", get(get_stats))
        .route("/v1/admin/registry/orphans", get(get_orphans))
        .route("/v1/admin/registry/eligible", get(get_eligible))
        .route(
            "/v1/admin/registry/mods/:alias_source/:external_key",
            get(get_mod_versions),
        )
        .route(
            "/v1/admin/registry/mods/:alias_source/:external_key/uses",
            get(get_mod_uses),
        )
        .layer(from_fn_with_state(state.clone(), super::auth::require_auth))
        .with_state(state)
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

async fn get_mod_versions(
    State(state): State<AppState>,
    Path((alias_source, external_key)): Path<(String, String)>,
) -> Result<Json<Vec<VersionRow>>, ApiError> {
    Ok(Json(
        run_query(&state, move |c| {
            queries::versions_of_mod(c, &alias_source, &external_key)
        })
        .await?,
    ))
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
