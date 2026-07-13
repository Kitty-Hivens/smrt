//! HTTP layer (controllers): the public `/v1` read API, the gated `/v1` write +
//! authoring API (`/v1/registry`, `/v1/authoring`, `/v1/users`, ...), and the
//! shared response error. `router` assembles the full application router from
//! the halves.

pub mod admin;
pub mod auth;
pub mod error;
pub mod jobs;
pub mod member;
pub mod panel;
pub mod public;
pub mod registry;

pub use error::ApiError;

use crate::state::AppState;
use axum::Router;

/// The full application router: public reads, admin writes + authoring, build
/// jobs, the panel auth endpoints, and the embedded panel under `/admin`.
pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(public::router(state.clone()))
        .merge(admin::router(state.clone()))
        .merge(member::router(state.clone()))
        .merge(registry::router(state.clone()))
        .merge(auth::router(state.clone()))
        .merge(jobs::router(state.clone()))
        .merge(panel::router())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::str::FromStr;

    // Assembling the full router merges every sub-router into one matchit tree;
    // an overlapping route would panic here, which is exactly the startup crash
    // we want a test to catch rather than a deploy.
    // Community pack ids carry slashes (u/<uid>/<pack>); they ride in a
    // single `:pack_id` segment percent-encoded, so this pins that axum decodes
    // %2F back into the slashed id the handler sees. If this ever regresses, the
    // whole community-authoring URL scheme breaks.
    #[tokio::test]
    async fn path_param_decodes_percent_encoded_slashes() {
        use axum::body::Body;
        use axum::extract::Path;
        use axum::http::Request;
        use axum::routing::get;
        use tower::ServiceExt;

        async fn echo(Path(id): Path<String>) -> String {
            id
        }
        let app = Router::new().route("/p/:id", get(echo));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/p/u%2F42%2FMyPack")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"u/42/MyPack");
    }

    #[test]
    fn full_router_assembles_without_route_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            bind_addr: std::net::SocketAddr::from_str("127.0.0.1:0").unwrap(),
            storage_dir: dir.path().to_path_buf(),
            admin_token: None,
            cookie_secure: false,
            mirror_base: "http://localhost".into(),
            github_client_id: None,
            github_client_secret: None,
            admin_github_uids: Vec::new(),
        };
        let state = AppState::new(config).unwrap();
        let _ = router(state);
    }
}
