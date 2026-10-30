#![expect(
    clippy::same_name_method,
    reason = "the Embed derive generates methods with the same names as the trait"
)]

use std::collections::HashMap;
use std::sync::OnceLock;

use axum::{Router, body::Body, extract::Path, response::Response, routing::get};
use http::{StatusCode, header};
use rust_embed::Embed;
use serde::Deserialize;
use tower_http::services::ServeDir;

#[derive(Deserialize)]
struct ManifestEntry {
    file: String,
}

/// Vite's manifest is written by `build.rs` before Rust compiles, so embedding
/// it is safe and lets templates reference content-addressed files.
fn manifest() -> &'static HashMap<String, ManifestEntry> {
    static MANIFEST: OnceLock<HashMap<String, ManifestEntry>> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(include_str!("../static/vite/.vite/manifest.json")).unwrap_or_default()
    })
}

/// Resolve a Vite entry (e.g. `src/app.ts`) to its built `/static/vite/...` URL.
pub(crate) fn vite_asset(entry: &str) -> Option<String> {
    manifest()
        .get(entry)
        .map(|item| format!("/static/vite/{}", item.file))
}

trait ServeEmbed: Embed {
    fn serve_embedded(path: &str) -> Response {
        match Self::get(path) {
            Some(content) => {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    // long cache since Vite output is content-addressed if you hash filenames;
                    // drop this header if you're using the fixed filenames from the earlier scaffold
                    .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
                    .body(Body::from(content.data))
                    .expect("static response headers are valid")
            }
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("404 Not Found"))
                .expect("static response headers are valid"),
        }
    }
}

impl<T: Embed> ServeEmbed for T {}

#[derive(Embed)]
#[folder = "static/vite/assets"]
pub(crate) struct ViteAssets;

#[derive(Embed)]
#[folder = "static/vite/images"]
pub(crate) struct ViteImages;

pub(crate) fn serve_dir() -> Router {
    Router::new()
        .nest_service(
            "/static/vite/vendored",
            ServeDir::new("static/vite/vendored").precompressed_br(),
        )
        .route(
            "/static/vite/assets/{*path}",
            get(async |Path(path): Path<String>| ViteAssets::serve_embedded(&path)),
        )
        .route(
            "/static/vite/images/{*path}",
            get(async |Path(path): Path<String>| ViteImages::serve_embedded(&path)),
        )
}
