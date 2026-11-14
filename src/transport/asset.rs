#![expect(
    clippy::same_name_method,
    reason = "the Embed derive generates methods with the same names as the trait"
)]

use axum::{Router, body::Body, extract::Path, response::Response, routing::get};
use http::{StatusCode, header};
use rust_embed::Embed;
use tower_http::services::ServeDir;

use crate::prelude::*;

use super::cache;

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
                    .header(header::CACHE_CONTROL, cache::PUBLIC_IMMUTABLE)
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
struct ViteAssets;

#[derive(Embed)]
#[folder = "static/vite/images"]
struct ViteImages;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .nest_service(
            "/static/vite/vendored",
            ServeDir::new("static/vite/vendored").precompressed_br(),
        )
        .nest_service(
            "/static/vite/fonts",
            ServeDir::new("static/vite/fonts").precompressed_br(),
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
