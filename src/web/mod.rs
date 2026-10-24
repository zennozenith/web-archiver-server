use crate::AppState;

use axum::{Router, routing::get};

mod index;
mod pending;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index::index))
        .route("/counter", get(index::vite_counter))
        .route(
            "/pending",
            get(pending::get_pending).post(pending::create_pending),
        )
}
