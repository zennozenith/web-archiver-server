use crate::AppState;

use axum::{Router, routing::get};

mod error;
mod index;
mod pending;

pub use error::negotiate_errors;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index::index))
        .route("/counter", get(index::vite_counter))
        .route(
            "/pending",
            get(pending::get_pending).post(pending::create_pending),
        )
        .route(
            "/pending/{id}",
            // get(get_pending).patch(patch_pending).delete(delete_pending),
            axum::routing::delete(pending::delete_pending),
        )
        .with_state(state)
}
