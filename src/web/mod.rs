use crate::AppState;

use axum::{
    Router,
    routing::{delete, get, patch, post},
};

mod error;
mod index;
mod pages;
mod pending;
mod tags;

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
        .route("/tags", post(tags::create_tag))
        .route(
            "/tags/{id}",
            patch(tags::rename_tag).delete(tags::delete_tag),
        )
        // Tags attached to an archived page.
        .route("/pages/{id}", delete(pages::delete_page))
        .route("/pages/{id}/tags", post(pages::add_page_tag))
        .route("/pages/{id}/tags/{tag_id}", delete(pages::remove_page_tag))
        .layer(axum::middleware::from_fn(negotiate_errors))
        .with_state(state)
}
