//! Administration pages, mounted at `/admin`.

mod audit;
mod roles;
mod users;

use axum::{
    Router,
    response::Redirect,
    routing::{get, patch, post},
};

use crate::AppState;

pub(super) use audit::AuditItem;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(|| async { Redirect::to("/admin/users") }))
        .route("/users", get(users::page))
        .route("/users/table", get(users::table))
        .route(
            "/users/{public_id}",
            get(users::manage).delete(users::delete),
        )
        .route("/users/{public_id}/status", patch(users::set_status))
        .route(
            "/users/{public_id}/roles/{role}",
            post(users::grant).delete(users::revoke),
        )
        .route("/roles", get(roles::page))
        .route("/audit", get(audit::page))
        .route("/audit/rows", get(audit::rows))
}
