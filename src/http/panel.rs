//! Serve the built Svelte panel at the site root. Assets are embedded into the
//! binary at release build (`rust-embed`), so the deploy story stays "ship one
//! binary"; in debug builds rust-embed reads `web/dist` from disk so the panel
//! can be rebuilt without recompiling Rust.

use axum::Router;
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/*path", get(asset))
}

async fn index() -> Response {
    serve("index.html")
}

async fn asset(Path(path): Path<String>) -> Response {
    serve(&format!("assets/{path}"))
}

fn serve(path: &str) -> Response {
    match Assets::get(path) {
        Some(file) => (
            [(header::CONTENT_TYPE, mime_for(path))],
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
        "png" => "image/png",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}
