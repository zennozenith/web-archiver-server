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
mod collections;
mod dashboard;
mod error;
mod form;
mod library;
mod links;
mod pending;
mod tags;
mod view;
mod view_archive;

use axum::{Router, response::Redirect, routing::get};

use crate::AppState;

pub(crate) fn router() -> Router<AppState> {
    // No login required
    let public = Router::new()
        // .route("/healthz", get(health))
        .merge(collections::public_router())
        .merge(auth::router());

    // Login required
    let authed = Router::new()
        .route("/", get(dashboard::page))
        .route("/dashboard", get(|| async { Redirect::permanent("/") }))
        .merge(links::router())
        .merge(collections::router())
        .merge(pending::router())
        .merge(tags::router())
        .merge(account::router());

    // Admin role required (layer order: require_user first, then require_admin)
    let admin = Router::new().nest("/admin", admin::router());

    public.merge(authed).merge(admin)
    // .fallback(not_found)
}
