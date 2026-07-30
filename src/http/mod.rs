//! HTTP layer (controllers): the public `/v1` read API, the gated `/v1` write +
//! authoring API (`/v1/registry`, `/v1/authoring`, `/v1/users`, ...), and the
//! shared response error. `router` assembles the full application router from
//! the halves.

pub mod admin;
pub mod apidoc;
pub mod auth;
pub mod error;
pub mod etag;
pub mod jobs;
pub mod member;
pub mod page;
pub mod panel;
pub mod public;
pub mod registry;

pub use error::ApiError;

use crate::accounts::Identity;
use crate::state::AppState;
use axum::Router;
use tower_http::compression::{CompressionLayer, DefaultPredicate, Predicate};

/// Ceiling on a single request body, shared by every write path that takes a
/// whole file (cache jars, pack static assets, member uploads, the bootstrap
/// archive). One home rather than a copy per router.
///
/// It is a memory ceiling, not just a size gate: these handlers extract `Bytes`,
/// so axum buffers the entire body in RAM before the handler runs, and the
/// bootstrap path copies it once more. A request near this limit holds that much
/// (bootstrap: twice that) for its lifetime. Sized for a whole instance archive
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

/// Response compression for everything the mirror answers in text.
///
/// It lives here rather than in the reverse proxy because the mirror is the
/// product and the proxy is one deployment's choice: a self-hoster behind
/// Caddy, behind nothing, or behind a proxy nobody configured would otherwise
/// ship manifests and registry listings as plain text. A manifest is tens of
/// kilobytes of repetitive JSON, which is the shape that compresses best.
///
/// What is left alone matters as much as what is not. The default predicate
/// already skips tiny bodies, images and event streams -- compressing an SSE
/// body would hold each line in an encoder buffer instead of delivering it,
/// which is the one thing a live log must not do. Added to that: jars and the
/// archives, because a zip re-compresses to roughly its own size and the CPU
/// spent proving it is the whole cost.
fn compression() -> tower_http::compression::CompressionLayer<impl Predicate> {
    use tower_http::compression::predicate::NotForContentType;
    CompressionLayer::new().compress_when(
        DefaultPredicate::new()
            .and(NotForContentType::const_new("application/java-archive"))
            .and(NotForContentType::const_new("application/zip"))
            .and(NotForContentType::const_new("application/octet-stream"))
            .and(NotAPart),
    )
}

/// A range names bytes of the representation the client receives, so encoding
/// one after slicing it would describe the answer with the wrong ruler -- the
/// client would splice compressed bytes into a file at offsets counted in
/// plain ones.
#[derive(Clone, Copy)]
struct NotAPart;

impl Predicate for NotAPart {
    fn should_compress<B>(&self, response: &axum::http::Response<B>) -> bool
    where
        B: axum::body::HttpBody,
    {
        response.status() != axum::http::StatusCode::PARTIAL_CONTENT
            && !response
                .headers()
                .contains_key(axum::http::header::CONTENT_RANGE)
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
        // inside compression, so a tag names the answer rather than one
        // encoding of it, and a 304 leaves nothing to encode
        .layer(axum::middleware::from_fn(etag::tag_json))
        .layer(compression())
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
        let app = Router::new().route("/p/{id}", get(echo));
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

    // A wildcard segment is `{*name}`; `{{` and `}}` are how axum 0.8 escapes a
    // literal brace. Written `{{*name}}` a route quietly stops matching anything
    // real and every request for it falls through to the SPA -- which is a 200
    // page of HTML where an asset or a file was asked for, not an error anyone
    // notices. Both wildcard routes are pinned here by behaviour: reaching the
    // handler at all is the assertion.
    #[tokio::test]
    async fn wildcard_routes_reach_their_handlers() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

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
        let app = router(AppState::new(config).unwrap());

        // A panel asset that does not exist answers 404 from the asset handler.
        // Escaped, this request would instead be served the app shell: 200 HTML.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/assets/nothing-here.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "an /assets/ request must reach the asset handler, not the SPA fallback"
        );

        // Same for a pack's static tree, one level deep so only a wildcard can
        // match it. The handler answers the API's error envelope; the fallback
        // answers plain text, so the body says which one replied.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/packs/Ghost/static/_nexira/icon.png")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("error").cloned())
                .is_some(),
            "a pack-static request must reach the handler; got {:?}",
            String::from_utf8_lossy(&body)
        );
    }

    /// Ask a one-route service wearing the real compression layer for a body of
    /// `content_type`, and report what it came back encoded as.
    async fn encoding_of(content_type: &'static str) -> Option<String> {
        use axum::body::Body;
        use axum::http::{Request, header};
        use axum::response::IntoResponse;
        use axum::routing::get;
        use tower::ServiceExt;

        // comfortably past the layer's minimum size, and compressible
        let payload = "smrt".repeat(64);
        let app = Router::new()
            .route(
                "/x",
                get(move || async move {
                    ([(header::CONTENT_TYPE, content_type)], payload).into_response()
                }),
            )
            .layer(compression());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/x")
                    .header(header::ACCEPT_ENCODING, "gzip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        resp.headers()
            .get(header::CONTENT_ENCODING)
            .map(|v| v.to_str().unwrap().to_string())
    }

    #[tokio::test]
    async fn json_travels_compressed() {
        assert_eq!(
            encoding_of("application/json").await.as_deref(),
            Some("gzip")
        );
    }

    // A jar is a zip: re-compressing it buys nothing and costs a pass over every
    // byte the mirror serves, which is the bulk of its traffic.
    #[tokio::test]
    async fn an_archive_is_served_as_it_lies() {
        assert_eq!(encoding_of("application/java-archive").await, None);
        assert_eq!(encoding_of("application/zip").await, None);
        assert_eq!(encoding_of("application/octet-stream").await, None);
    }

    // The one case where compression would not just waste work but break the
    // feature: a build log delivered a line at a time must not be held back to
    // fill an encoder's buffer.
    #[tokio::test]
    async fn a_live_stream_is_never_compressed() {
        assert_eq!(encoding_of("text/event-stream").await, None);
    }

    // A resumed download splices the answer into a file at the offsets it asked
    // for. Encoding the slice after cutting it would count those offsets in one
    // ruler and deliver bytes measured in another.
    #[tokio::test]
    async fn a_partial_answer_is_never_compressed() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode, header};
        use axum::response::IntoResponse;
        use axum::routing::get;
        use tower::ServiceExt;

        let payload = "smrt".repeat(64);
        let app = Router::new()
            .route(
                "/x",
                get(move || async move {
                    (
                        StatusCode::PARTIAL_CONTENT,
                        [
                            (header::CONTENT_TYPE, "text/plain"),
                            (header::CONTENT_RANGE, "bytes 0-255/1024"),
                        ],
                        payload,
                    )
                        .into_response()
                }),
            )
            .layer(compression());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/x")
                    .header(header::ACCEPT_ENCODING, "gzip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(resp.headers().get(header::CONTENT_ENCODING).is_none());
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
