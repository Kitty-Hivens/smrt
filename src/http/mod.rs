//! HTTP layer (controllers): the public `/v1` read API, the gated `/v1` write +
//! authoring API (`/v1/registry`, `/v1/authoring`, `/v1/users`, ...), and the
//! shared response error. `router` assembles the full application router from
//! the halves.

pub mod admin;
pub mod apidoc;
pub mod auth;
pub mod error;
pub mod jobs;
pub mod member;
pub mod panel;
pub mod public;
pub mod registry;

pub use error::ApiError;

use crate::accounts::Identity;
use crate::state::AppState;
use axum::Router;

/// Ceiling on a single request body, shared by every write path that takes a
/// whole file (cache jars, pack static assets, member uploads, the bootstrap
/// archive). One home rather than a copy per router.
///
/// It is a memory ceiling, not just a size gate: these handlers extract `Bytes`,
/// so axum buffers the entire body in RAM before the handler runs, and the
/// bootstrap path copies it once more. A request near this limit holds that much
/// (bootstrap: twice that) for its lifetime. Sized for a whole SC pack archive
/// uploaded in one shot; nginx in front is raised to match (see the deploy
/// config), since the smaller of the two wins.
pub(crate) const MAX_UPLOAD_BODY: usize = 8 * 1024 * 1024 * 1024;

/// Best-effort audit write shared by the admin and registry write paths: record
/// who did what. A failure is logged, never raised -- the audited action already
/// happened, so a lost trail entry must not turn a successful operation into an
/// error for the caller.
pub(crate) async fn audit(
    state: &AppState,
    who: &Identity,
    action: &str,
    target: Option<&str>,
    detail: Option<&str>,
) {
    let acc = state.accounts.clone();
    let (uid, login, action) = (who.uid, who.login.clone(), action.to_string());
    let (target, detail) = (target.map(String::from), detail.map(String::from));
    let res = tokio::task::spawn_blocking(move || {
        acc.record_audit(uid, &login, &action, target.as_deref(), detail.as_deref())
    })
    .await;
    match res {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(error = %e, "audit write failed"),
        Err(e) => tracing::warn!(error = %e, "audit task failed"),
    }
}

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
        .merge(apidoc::router())
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
            debug_token: None,
            debug_github_uids: Vec::new(),
        };
        let state = AppState::new(config).unwrap();
        let _ = router(state);
    }
}
