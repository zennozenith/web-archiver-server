mod actions;
// mod auth_pages;
mod fragments;
mod pages;
mod templates;
mod view;

use axum::middleware;
use axum::{
    Router,
    response::Redirect,
    routing::{get, post, put},
};

use crate::{AppState, error::AppError, web_archive::error::negotiate_errors};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        // .merge(auth_pages::router())
        .route("/", get(root))
        .route("/dashboard", get(pages::dashboard))
        // links
        .route("/links", get(pages::links).post(actions::create_link))
        .route("/links/search", get(fragments::search_links))
        .route(
            "/links/{id}",
            get(pages::link_detail)
                .put(actions::update_link)
                .delete(actions::delete_link),
        )
        .route("/links/{id}/pin", post(actions::toggle_pin))
        .route("/links/{id}/refetch", post(fragments::refetch_formats))
        .route("/links/{id}/format", get(fragments::link_format))
        .route("/links/{id}/highlights", post(actions::add_highlight))
        .route("/links/{id}/archive/{format}", get(pages::archive))
        .route("/pinned", get(pages::pinned))
        // collections
        .route(
            "/collections",
            get(pages::collections).post(actions::create_collection),
        )
        .route("/collections/new", get(pages::collection_new))
        .route("/collections/unorganized", get(pages::unorganized))
        .route(
            "/collections/{id}",
            get(pages::collection)
                .post(actions::update_collection)
                .put(actions::update_collection)
                .delete(actions::delete_collection),
        )
        .route("/collections/{id}/edit", get(pages::collection_edit))
        .route("/collections/{id}/sharing", put(actions::update_sharing))
        .route("/public/collections/{id}", get(pages::public_collection))
        // pending queue
        .route(
            "/pending",
            get(pages::pending)
                .post(actions::create_pending)
                .delete(actions::clear_pending),
        )
        .route("/pending/bulk", post(actions::bulk_create_pending))
        .route(
            "/pending/{id}",
            post(actions::update_pending).delete(actions::delete_pending),
        )
        .route("/pending/{id}/retry", post(actions::retry_pending))
        // tags
        .route("/tags", get(pages::tags))
        .route("/tags/search", get(fragments::search_tags))
        .route(
            "/tags/{key}",
            get(pages::tag)
                .put(actions::rename_tag)
                .delete(actions::delete_tag),
        )
        // export (import is left to the API for now)
        .route("/export", get(pages::export))
        .fallback(not_found)
        .layer(middleware::from_fn(negotiate_errors))
}

async fn root() -> Redirect {
    Redirect::to("/dashboard")
}

async fn not_found() -> AppError {
    AppError::NotFound("page")
}
