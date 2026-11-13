//! Server-rendered HTML routes.
//!
//! Stack: askama templates, htmx 4, daisyUI 5 + Tailwind 4, Alpine.js.
//! Handlers here only translate HTTP to `service::*` calls and service
//! results to view-models; permission checks, safety rules and audit entries
//! live in the service layer.
//!
//! | Area        | Module        | Error type                        |
//! |-------------|---------------|-----------------------------------|
//! | sign in etc.| [`auth`]      | `WebError` (inline alert)         |
//! | `/admin/*`  | [`admin`]     | `PageError` / `ActionError`       |
//! | `/account/*`| [`account`]   | `PageError` / `ActionError`       |
//! | `/`         | [`dashboard`] | `PageError`                       |

mod account;
mod admin;
mod alert;
mod auth;
mod error;
mod form;
mod view;

// mod actions;
// mod error;
// mod serve;
// mod user;
// mod web_archive;
// mod fragments;
// mod pages;
// mod templates;

use axum::{Router, response::Redirect, routing::get};

use crate::AppState;

pub(crate) fn router() -> Router<AppState> {
    // No login required
    let public = Router::new()
        // .route("/healthz", get(health))
        // .route("/shared/{token}", get(collections::shared)) // public collection share link
        .merge(auth::router());

    // Login required
    let authed = Router::new()
        // .route("/", get(dashboard::page)) // archive dashboard
        .route("/dashboard", get(|| async { Redirect::permanent("/") }))
        // .nest("/links", links::router())
        // .nest("/collections", collections::router())
        // .nest("/pending", pending::router())
        // .nest("/tags", tags::router())
        // .nest("/archive", web_archive::router())
        .merge(account::router());

    // Admin role required (layer order: require_user first, then require_admin)
    let admin = Router::new().nest("/admin", admin::router());

    public.merge(authed).merge(admin)
    // .fallback(not_found)
}

/*
Routes structure to be implemented


links (nested at /links)
rust
Router::new()
    .route("/", get(pages::index).post(actions::create))
    .route("/search", get(fragments::search))
    .route("/pinned", get(pages::pinned))
    .route("/{public_id}",
        get(pages::detail).put(actions::update).delete(actions::delete))
    .route("/{public_id}/pin", put(actions::pin).delete(actions::unpin))
    .route("/{public_id}/formats", post(actions::refetch_formats))
    .route("/{public_id}/formats/{format}", get(fragments::format))
    .route("/{public_id}/read/{format}", get(pages::read))
    .route("/{public_id}/highlights", post(actions::add_highlight))
    .route("/{public_id}/highlights/{highlight_id}",
        put(actions::update_highlight).delete(actions::delete_highlight))

collections (nested at /collections)
Router::new()
    .route("/", get(pages::index).post(actions::create))
    .route("/new", get(pages::new))
    .route("/unorganized", get(pages::unorganized))
    .route("/{public_id}",
        get(pages::show).put(actions::update).delete(actions::delete))
    .route("/{public_id}/edit", get(pages::edit))
    .route("/{public_id}/sharing",
        put(actions::enable_sharing).delete(actions::disable_sharing))

pending (nested at /pending)
Router::new()
    .route("/", get(pages::index).post(actions::create).delete(actions::clear))
    .route("/bulk", post(actions::bulk_create))
    .route("/{public_id}", put(actions::update).delete(actions::delete))
    .route("/{public_id}/retry", post(actions::retry))

tags (nested at /tags)
Router::new()
    .route("/", get(pages::index))
    .route("/search", get(fragments::search))
    .route("/{slug}", get(pages::show).put(actions::rename).delete(actions::delete))

account (merged, absolute paths as in the demo)
Router::new()
    .route("/account", get(|| async { Redirect::to("/account/profile") }))
    .route("/account/profile", get(profile::page).put(profile::update))
    .route("/account/sessions", get(sessions::page))
    .route("/account/sessions/{public_id}", delete(sessions::revoke))
    .route("/account/keys", get(keys::page).post(keys::create))
    .route("/account/keys/{public_id}", delete(keys::revoke))
    .route("/account/export", get(export::download))   // moved from /export
    // later: .route("/account/import", get(import::page).post(import::upload))

admin (nested at /admin)
Router::new()
    .route("/", get(|| async { Redirect::to("/admin/users") }))
    .route("/users", get(users::page))
    .route("/users/table", get(users::table))
    .route("/users/{public_id}", get(users::manage).delete(users::delete))
    .route("/users/{public_id}/status", patch(users::set_status))
    .route("/users/{public_id}/roles/{role}", post(users::grant).delete(users::revoke))
    .route("/roles", get(roles::page))
    .route("/audit", get(audit::page))
    .route("/audit/rows", get(audit::rows))
    // optional, since you have a fetch queue:
    // .route("/jobs", get(jobs::page))
    // .route("/jobs/rows", get(jobs::rows))
*/
