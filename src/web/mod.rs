use crate::AppState;

use axum::{
    Router,
    routing::{delete, get, patch, post},
};

mod error;
mod index;
mod page;
mod pending;
mod tag;

use error::negotiate_errors;

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index::index))
        .route("/counter", get(index::vite_counter))
        .route(
            "/pending",
            get(pending::get_pending).post(pending::create_pending),
        )
        .route("/pending/{id}", delete(pending::delete_pending))
        // Tags shown in the sidebar.
        .route("/tags", post(tag::create_tag))
        .route("/tags/{id}", patch(tag::rename_tag).delete(tag::delete_tag))
        // Tags attached to an archived page.
        .route("/pages/{id}", delete(page::delete_page))
        .route("/pages/{id}/tags", post(page::add_page_tag))
        .route("/pages/{id}/tags/{tag_id}", delete(page::remove_page_tag))
        .layer(axum::middleware::from_fn(negotiate_errors))
        .with_state(state)
}
