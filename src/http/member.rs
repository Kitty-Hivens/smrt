//! Member-accessible API: endpoints any signed-in user may call, gated by
//! `require_session` and scoped to what they own. Distinct from `admin`, which
//! requires the admin role -- this is the member tier of the ladder.

use super::ApiError;
use crate::accounts::Identity;
use crate::domain::PackSummary;
use crate::state::AppState;
use axum::extract::State;
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use axum::{Extension, Json, Router};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/me/packs", get(my_packs))
        .route("/v1/me/authoring", get(my_authoring))
        .layer(from_fn_with_state(
            state.clone(),
            super::auth::require_session,
        ))
        .with_state(state)
}

/// The caller's own packs -- the "my packs" view. Draft and community packs the
/// public `/v1/packs` listing hides show here for their owner; an admin sees all.
async fn my_packs(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<Vec<PackSummary>>, ApiError> {
    let mine = state
        .storage
        .list_pack_summaries()
        .await?
        .into_iter()
        .filter(|p| identity.owns_or_admin(p.owner))
        .collect();
    Ok(Json(mine))
}

/// The caller's own authoring pack ids, including unbuilt drafts that have no
/// summary yet. The "my packs" list unions this with the built summaries so a
/// freshly-created draft is reachable before its first build.
async fn my_authoring(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<Vec<String>>, ApiError> {
    let mine = state
        .storage
        .list_authoring_packs()
        .await?
        .into_iter()
        .filter(|id| super::auth::may_author(&identity, id))
        .collect();
    Ok(Json(mine))
}
