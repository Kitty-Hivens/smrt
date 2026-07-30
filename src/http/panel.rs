//! Serve the built Svelte panel at the site root. Assets are embedded into the
//! binary at release build (`rust-embed`), so the deploy story stays "ship one
//! binary"; in debug builds rust-embed reads `web/dist` from disk so the panel
//! can be rebuilt without recompiling Rust.

use axum::Router;
use axum::extract::Path;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/{*path}", get(asset))
        // Root-level files, routed explicitly rather than by a wildcard: anything
        // not routed is an app path, and a pack id may carry a dot, so "looks
        // like a filename" cannot decide which is which. `.ico` is here because
        // bookmark bars, link previews and older browsers ask for that name
        // whatever the shell says -- and were being handed the app shell.
        .route("/favicon.svg", get(|| favicon("favicon.svg")))
        .route("/favicon.png", get(|| favicon("favicon.png")))
        .route("/favicon.ico", get(|| favicon("favicon.ico")))
        .route(
            "/apple-touch-icon.png",
            get(|| favicon("apple-touch-icon.png")),
        )
        // The panel owns its URLs: a section is a path, and a mod page is a
        // shareable link. Any path the API does not claim serves the app shell,
        // which then reads the URL -- so a reload, a bookmark or the mouse's
        // back button land where they should instead of on a 404.
        .fallback(get(spa))
}

/// Serve the panel for an unclaimed path, but never for the API surface: a bad
/// `/v1` path must stay a 404 a client can parse, not a page of HTML.
async fn spa(uri: Uri) -> Response {
    if uri.path().starts_with("/v1") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    serve("index.html")
}

async fn index() -> Response {
    serve("index.html")
}

/// The site icon, in the three shapes something might ask for. Cacheable for a
/// day: it is not content-addressed like the hashed assets, but it also does not
/// change between deploys the way the shell does.
async fn favicon(name: &str) -> Response {
    match Assets::get(name) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, mime_for(name)),
                (header::CACHE_CONTROL, "public, max-age=86400"),
            ],
            file.data.into_owned(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A built asset. Missing means the deploy moved on: the panel's assets carry a
/// content hash in the name, so an asset that is not here is one no current page
/// should be asking for. Answered as a bare 404 rather than with a body, because
/// a body makes the browser report a module load as a MIME-type error and hide
/// the 404 underneath it.
async fn asset(Path(path): Path<String>) -> Response {
    let path = format!("assets/{path}");
    match Assets::get(&path) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, mime_for(&path)),
                (header::CACHE_CONTROL, IMMUTABLE),
            ],
            file.data.into_owned(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// An asset's name carries a hash of its contents, so the file at a given name
/// never changes and the browser need never ask about it again.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// The shell, on the other hand, is the map to those names, and a stale map is
/// how a browser ends up asking for a chunk that no longer exists: the page
/// loads, and the first thing it lazily imports 404s. With no header at all a
/// browser is free to cache it by guesswork, which is exactly what it did.
/// `no-cache` keeps the copy but revalidates it, so a deploy is picked up on the
/// next load rather than whenever the guess expires.
const REVALIDATE: &str = "no-cache";

fn serve(path: &str) -> Response {
    match Assets::get(path) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, mime_for(path)),
                (header::CACHE_CONTROL, REVALIDATE),
            ],
            file.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "ico" => "image/vnd.microsoft.icon",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn get(uri: &str) -> Response {
        router()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn caching(resp: &Response) -> Option<&str> {
        resp.headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap())
    }

    // The shell names the hashed assets, so a browser holding an old copy asks
    // for chunks a later deploy no longer has. Left to its own devices it caches
    // the shell by guesswork, which is how a page loads and then fails on the
    // first thing it lazily imports.
    #[tokio::test]
    async fn the_shell_is_revalidated_and_the_assets_are_not() {
        let shell = get("/").await;
        assert_eq!(shell.status(), StatusCode::OK);
        assert_eq!(caching(&shell), Some(REVALIDATE));

        // whatever the built asset happens to be called this build
        let name = Assets::iter()
            .map(|f| f.to_string())
            .find(|f| f.starts_with("assets/") && f.ends_with(".js"))
            .expect("a built panel has at least one script");
        let asset = get(&format!("/{name}")).await;
        assert_eq!(asset.status(), StatusCode::OK);
        assert_eq!(caching(&asset), Some(IMMUTABLE));
    }

    // Something always asks for /favicon.ico by name -- a bookmark bar, a link
    // preview, an older browser -- whatever the shell declares. Handed the app
    // shell it shows nothing, which is what happened: an icon on one browser and
    // none anywhere else.
    #[tokio::test]
    async fn the_icon_is_an_image_in_every_shape_something_asks_for() {
        for (path, content_type) in [
            ("/favicon.svg", "image/svg+xml"),
            ("/favicon.png", "image/png"),
            ("/favicon.ico", "image/vnd.microsoft.icon"),
            ("/apple-touch-icon.png", "image/png"),
        ] {
            let resp = get(path).await;
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            assert_eq!(
                resp.headers().get(header::CONTENT_TYPE).unwrap(),
                content_type,
                "{path} must not come back as the app shell"
            );
            // and it is not re-fetched on every page load the way the shell is
            assert_eq!(caching(&resp), Some("public, max-age=86400"), "{path}");
        }
    }

    // A chunk from a deploy that has moved on must read as gone, not as a file
    // of the wrong type: with a body, a module request reports a MIME error and
    // the 404 underneath it never surfaces.
    #[tokio::test]
    async fn a_chunk_that_no_longer_exists_is_plainly_gone() {
        let resp = get("/assets/ModManager-fromLastDeploy.js").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(
            resp.headers().get(header::CONTENT_TYPE).is_none(),
            "no body, so nothing for the module loader to mistype"
        );
    }
}
