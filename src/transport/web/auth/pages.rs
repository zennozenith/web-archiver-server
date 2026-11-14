//! GET handlers: render the auth pages.

use askama::Template;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use super::HOME;
use crate::{
    error::AppError,
    identity::AuthUser,
    prelude::*,
    transport::{extract::body::AppQuery, htmx::Render},
};

// ------------------------------------------------------------------ templates

#[derive(Debug, Template)]
#[template(path = "login.templ.html")]
struct LoginTpl {
    title: &'static str,
}

#[derive(Debug, Template)]
#[template(path = "register.templ.html")]
struct RegisterTpl {
    title: &'static str,
}

#[derive(Debug, Template)]
#[template(path = "verify.templ.html")]
struct VerifyTpl {
    title: &'static str,
    token: String,
}

#[derive(Debug, Template)]
#[template(path = "forgot.templ.html")]
struct ForgotTpl {
    title: &'static str,
}

#[derive(Debug, Template)]
#[template(path = "reset.templ.html")]
struct ResetTpl {
    title: &'static str,
    token: String,
}

// -------------------------------------------------------------------- helpers

/// Query string of the e-mailed links: `?token=...`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct TokenQuery {
    #[serde(default)]
    token: String,
}

/// A malformed query string just means "no token"; the user can still type it.
fn token_from(AppQuery(q): AppQuery<TokenQuery>) -> String {
    q.token
}

/// Renders `page`, or sends already signed-in users home.
fn guest_only(user: &Result<AuthUser, AppError>, page: impl Template) -> Response {
    if user.is_ok() {
        Redirect::to(HOME).into_response()
    } else {
        Render(page).into_response()
    }
}

// ------------------------------------------------------------------- handlers

pub(super) async fn login(user: Result<AuthUser, AppError>) -> Response {
    guest_only(&user, LoginTpl { title: "Sign in" })
}

pub(super) async fn register(user: Result<AuthUser, AppError>) -> Response {
    guest_only(
        &user,
        RegisterTpl {
            title: "Create account",
        },
    )
}

pub(super) async fn forgot() -> Response {
    Render(ForgotTpl {
        title: "Forgot password",
    })
    .into_response()
}

pub(super) async fn verify(query: AppQuery<TokenQuery>) -> Response {
    Render(VerifyTpl {
        title: "Verify e-mail",
        token: token_from(query),
    })
    .into_response()
}

pub(super) async fn reset(query: AppQuery<TokenQuery>) -> Response {
    Render(ResetTpl {
        title: "Reset password",
        token: token_from(query),
    })
    .into_response()
}
