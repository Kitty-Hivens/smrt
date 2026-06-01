//! HTTP layer (controllers): the public `/v1` read API, the `/v1/admin`
//! write + authoring API, and the shared response error. `router` assembles
//! the full application router from the two halves.

pub mod admin;
pub mod auth;
pub mod error;
pub mod jobs;
pub mod panel;
pub mod public;

pub use error::ApiError;

use crate::state::AppState;
use axum::Router;

/// The full application router: public reads, admin writes + authoring, build
/// jobs, the panel auth endpoints, and the embedded panel under `/admin`.
pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(public::router(state.clone()))
        .merge(admin::router(state.clone()))
        .merge(auth::router(state.clone()))
        .merge(jobs::router(state.clone()))
        .merge(panel::router())
}
