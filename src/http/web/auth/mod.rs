//! Public auth pages: sign in, register, verify e-mail, forgot / reset password.
//!
//! * [`pages`]: GET handlers that render a full page.
//! * [`actions`]: POST handlers, called by htmx. Success redirects with
//!   `HX-Redirect` or shows a success alert; failure returns a
//!   [`WebError`](super::error::WebError) alert that htmx drops into
//!   `#form-alert`.
//!
//! All business rules (lockout, password history, audit) live in
//! `service::auth`; nothing here duplicates them.

mod actions;
mod pages;
mod session;

use axum::{
    Router,
    routing::{get, post},
};

use crate::AppState;

/// Where a signed-in user lands.
const HOME: &str = "/";
/// The sign-in page.
const LOGIN: &str = "/login";

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/login", get(pages::login).post(actions::login))
        .route("/register", get(pages::register).post(actions::register))
        .route("/verify", get(pages::verify).post(actions::verify))
        .route("/forgot", get(pages::forgot).post(actions::forgot))
        .route("/reset", get(pages::reset).post(actions::reset))
        .route("/logout", post(actions::logout))
}
