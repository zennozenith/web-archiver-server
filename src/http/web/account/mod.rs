//! "My account": profile, sessions and API keys (self-service).

mod keys;
mod overview;
mod profile;
mod sessions;

use axum::{
    Router,
    routing::{delete, get},
};

use crate::AppState;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/account", get(overview::page))
        .route("/account/profile", get(profile::page).put(profile::update))
        .route("/account/sessions", get(sessions::page))
        .route("/account/sessions/{public_id}", delete(sessions::revoke))
        .route("/account/keys", get(keys::page).post(keys::create))
        .route("/account/keys/{public_id}", delete(keys::revoke))
}
