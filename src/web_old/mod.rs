use crate::AppState;

use axum::{Router, routing::get};

pub(crate) mod error;
mod index;
mod page;
mod pending;
mod tag;

use error::negotiate_errors;

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index::index))
        .route("/counter", get(index::vite_counter))
        .layer(axum::middleware::from_fn(negotiate_errors))
        .with_state(state)
}
