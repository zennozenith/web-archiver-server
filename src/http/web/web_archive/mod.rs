use crate::AppState;

use axum::{
    Router,
    routing::{delete, get, patch, post},
};

pub(crate) mod error;
mod index;
mod page;
mod pending;
mod tag;

use error::negotiate_errors;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index::index))
        .route("/pending", post(pending::create_pending))
        .route("/pending/{id}", delete(pending::delete_pending))
        .route("/page/{id}/tag", post(page::add_page_tag))
        .route("/tag", post(tag::create_tag))
        .route("/tag/{id}", patch(tag::rename_tag).delete(tag::delete_tag))
        .route("/counter", get(index::vite_counter))
        .layer(axum::middleware::from_fn(negotiate_errors))
}
