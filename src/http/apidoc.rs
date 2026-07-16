//! OpenAPI documentation for the mirror's HTTP API.
//!
//! The spec is derived from the handlers themselves (the `#[utoipa::path]`
//! attributes and `ToSchema` derives on the wire types), so it tracks the code
//! rather than a hand-maintained document. It is served two ways: the raw
//! `/openapi.json` for tooling, and `/docs`, a Scalar reference UI.
//!
//! Scalar's browser bundle is vendored and served from this binary
//! (`/docs/scalar.js`), not loaded from a CDN -- the docs page makes no external
//! request, in keeping with the mirror's self-hosted, no-phone-home stance.

use axum::http::header;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::{Json, Router};
use utoipa::OpenApi;

use crate::domain::pack::{CommunityPack, PackListing, PackSummary, PackTier, Visibility};
use crate::domain::server::{Featured, Health, ServerEntry, ServerListing};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "smrt mirror API",
        version = "1",
        description = "Read and authoring API for the smrt Minecraft mod mirror. \
                       The public surface below is what the launcher and the site \
                       consume; member, operator and debug surfaces are added as \
                       they are annotated."
    ),
    paths(
        crate::http::public::health,
        crate::http::public::list_packs,
        crate::http::public::get_pack_summary,
        crate::http::public::list_community,
        crate::http::public::list_servers,
        crate::http::public::get_featured,
    ),
    components(schemas(
        Health,
        PackListing,
        PackSummary,
        PackTier,
        Visibility,
        CommunityPack,
        ServerListing,
        ServerEntry,
        Featured,
    )),
    tags((name = "public", description = "Unauthenticated reads: the launcher catalog, servers, featured set."))
)]
struct ApiDoc;

pub fn router() -> Router {
    Router::new()
        .route("/openapi.json", get(openapi_json))
        .route("/docs", get(docs_page))
        .route("/docs/scalar.js", get(scalar_js))
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

// Vendored Scalar standalone bundle -- served locally so the docs page never
// reaches a CDN.
const SCALAR_JS: &str = include_str!("../../vendor/scalar.standalone.js");

async fn scalar_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        SCALAR_JS,
    )
}

// `withDefaultFonts:false` stops Scalar pulling Inter/mono from fonts.scalar.com;
// it falls back to the system stack. The CSP below is the hard guarantee: even if
// the bundle tries to reach fonts.scalar.com or api.scalar.com (its hosted search),
// the browser refuses -- connect-src and font-src are 'self' only, so the docs
// page contacts nothing but this mirror.
const DOCS_HTML: &str = r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>smrt mirror API</title>
  </head>
  <body>
    <script
      id="api-reference"
      data-url="/openapi.json"
      data-configuration='{"withDefaultFonts":false,"darkMode":true}'
    ></script>
    <script src="/docs/scalar.js"></script>
  </body>
</html>
"#;

const DOCS_CSP: &str = "default-src 'self'; \
    script-src 'self' 'unsafe-inline' 'unsafe-eval'; \
    style-src 'self' 'unsafe-inline'; \
    img-src 'self' data: blob:; \
    font-src 'self' data:; \
    connect-src 'self'";

async fn docs_page() -> impl IntoResponse {
    (
        [(header::CONTENT_SECURITY_POLICY, DOCS_CSP)],
        Html(DOCS_HTML),
    )
}
