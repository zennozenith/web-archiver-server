use axum::{
    Router,
    body::Body,
    extract::Path,
    http::{StatusCode, header},
    response::Response,
    routing::get,
};
use rust_embed::RustEmbed;
use tower_http::services::ServeDir;

trait ServeEmbed: RustEmbed {
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
                    .unwrap()
            }
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("404 Not Found"))
                .unwrap(),
        }
    }
}

impl<T: RustEmbed> ServeEmbed for T {}

#[derive(RustEmbed)]
#[folder = "static/vite/assets"]
pub struct ViteAssets;

#[derive(RustEmbed)]
#[folder = "static/vite/images"]
pub struct ViteImages;

// pub async fn static_handler(Path(path): Path<String>) -> impl IntoResponse {
//     serve_embedded(&path)
// }

pub(super) fn serve_dir() -> Router {
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
