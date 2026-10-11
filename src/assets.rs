use axum::{
    Router,
    body::Body,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rust_embed::RustEmbed;
use tower_http::services::ServeDir;

#[derive(RustEmbed)]
#[folder = "static/vite"]
pub struct AssetsVite;

pub async fn static_handler(Path(path): Path<String>) -> impl IntoResponse {
    serve_embedded(&path)
}

fn serve_embedded(path: &str) -> Response {
    match AssetsVite::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                // long cache since Vite output is content-addressed if you hash filenames;
                // drop this header if you're using the fixed filenames from the earlier scaffold
                .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
                .body(Body::from(content.data))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404 Not Found"))
            .unwrap(),
    }
}

pub(super) fn serve_dir() -> Router {
    Router::new()
        .route("/static/vite/{*path}", get(static_handler))
        .nest_service("/static", ServeDir::new("static"))
}
