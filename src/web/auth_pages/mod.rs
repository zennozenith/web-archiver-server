//! Public pages: sign in, register, verify e-mail, forgot / reset password.
//!
//! The forms post to these routes via htmx; the work is delegated to the JSON
//! handlers in `routes::auth`, so lockout, password-history and audit rules
//! are not duplicated. Success redirects (`HX-Redirect`); failures answer with
//! an alert fragment that htmx drops into `#form-alert`.
//!

use std::net::SocketAddr;

use askama::Template;
use axum::{
    Form, Json, Router,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::Value;

use super::extract::{ActionError, Fragments, Render, WebRejection, WebUser, with_redirect};
use crate::{
    AppState,
    error::AppError,
    routes::auth::{self as flow, ForgotReq, LoginReq, RegisterReq, ResetReq, TokenReq},
};

// ------------------------------------------------------------------ templates

#[derive(Debug, Template)]
#[template(path = "login.html")]
struct LoginTpl {
    title: &'static str,
}

#[derive(Debug, Template)]
#[template(path = "register.html")]
struct RegisterTpl {
    title: &'static str,
}

#[derive(Debug, Template)]
#[template(path = "verify.html")]
struct VerifyTpl {
    title: &'static str,
    token: String,
}

#[derive(Debug, Template)]
#[template(path = "forgot.html")]
struct ForgotTpl {
    title: &'static str,
}

#[derive(Debug, Template)]
#[template(path = "reset.html")]
struct ResetTpl {
    title: &'static str,
    token: String,
}

#[derive(Debug, Template)]
#[template(path = "partials/alert.html")]
struct AlertTpl {
    kind: &'static str,
    message: String,
    href: String,
    label: &'static str,
}

// -------------------------------------------------------------------- alerts

fn alert(status: StatusCode, tpl: &AlertTpl) -> Response {
    Fragments::new()
        .status(status)
        .push(tpl)
        .map_or_else(IntoResponse::into_response, IntoResponse::into_response)
}

fn fail(error: &AppError) -> Response {
    let (_, message) = error.parts();
    alert(
        StatusCode::UNPROCESSABLE_ENTITY,
        &AlertTpl {
            kind: "alert-error",
            message,
            href: String::new(),
            label: "",
        },
    )
}

fn done(message: &str, href: String, label: &'static str) -> Response {
    alert(
        StatusCode::OK,
        &AlertTpl {
            kind: "alert-success",
            message: message.to_owned(),
            href,
            label,
        },
    )
}

fn str_field<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    body.get(key).and_then(Value::as_str)
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenQuery {
    #[serde(default)]
    token: String,
}

// --------------------------------------------------------------------- login

pub(crate) async fn login_page(signed_in: Result<WebUser, WebRejection>) -> Response {
    if signed_in.is_ok() {
        return Redirect::to("/").into_response();
    }
    Render(LoginTpl { title: "Sign in" }).into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct LoginForm {
    identifier: String,
    password: String,
}

pub(crate) async fn login_submit(
    State(st): State<AppState>,
    conn: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let req = LoginReq {
        identifier: form.identifier,
        password: form.password,
    };
    match flow::login(State(st), conn, headers, Json(req)).await {
        // `login` already carries the Set-Cookie header.
        Ok(resp) => with_redirect(resp, "/"),
        Err(e) => fail(&e),
    }
}

pub(crate) async fn logout(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ActionError> {
    let resp = flow::logout(State(st), headers).await?;
    Ok(with_redirect(resp, "/login"))
}

// ------------------------------------------------------------------ register

pub(crate) async fn register_page(signed_in: Result<WebUser, WebRejection>) -> Response {
    if signed_in.is_ok() {
        return Redirect::to("/").into_response();
    }
    Render(RegisterTpl {
        title: "Create account",
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegisterForm {
    email: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    display_name: String,
    password: String,
}

pub(crate) async fn register_submit(
    State(st): State<AppState>,
    Form(form): Form<RegisterForm>,
) -> Response {
    let req = RegisterReq {
        email: form.email,
        username: Some(form.username),
        password: form.password,
        display_name: Some(form.display_name),
    };
    match flow::register(State(st), Json(req)).await {
        Ok((_, Json(body))) => {
            let message = str_field(&body, "message").unwrap_or("Account created.");
            let (href, label) = match (
                str_field(&body, "dev_verify_token"),
                str_field(&body, "status"),
            ) {
                (Some(token), _) => (format!("/verify?token={token}"), "Verify e-mail"),
                (None, Some("active")) => ("/login".to_owned(), "Sign in"),
                (None, _) => ("/verify".to_owned(), "Enter token"),
            };
            done(message, href, label)
        }
        Err(e) => fail(&e),
    }
}

// -------------------------------------------------------------------- verify

pub(crate) async fn verify_page(Query(q): Query<TokenQuery>) -> Response {
    Render(VerifyTpl {
        title: "Verify e-mail",
        token: q.token,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenForm {
    token: String,
}

pub(crate) async fn verify_submit(
    State(st): State<AppState>,
    Form(form): Form<TokenForm>,
) -> Response {
    match flow::verify_email(State(st), Json(TokenReq { token: form.token })).await {
        Ok(_) => done(
            "E-mail verified. You can sign in now.",
            "/login".to_owned(),
            "Sign in",
        ),
        Err(e) => fail(&e),
    }
}

// ------------------------------------------------------------ forgot / reset

pub(crate) async fn forgot_page() -> Response {
    Render(ForgotTpl {
        title: "Forgot password",
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct ForgotForm {
    email: String,
}

pub(crate) async fn forgot_submit(
    State(st): State<AppState>,
    Form(form): Form<ForgotForm>,
) -> Response {
    match flow::forgot_password(State(st), Json(ForgotReq { email: form.email })).await {
        Ok(Json(body)) => {
            let message = str_field(&body, "message").unwrap_or("Check your inbox.");
            let (href, label) = str_field(&body, "dev_reset_token").map_or_else(
                || (String::new(), ""),
                |t| (format!("/reset?token={t}"), "Choose a new password"),
            );
            done(message, href, label)
        }
        Err(e) => fail(&e),
    }
}

pub(crate) async fn reset_page(Query(q): Query<TokenQuery>) -> Response {
    Render(ResetTpl {
        title: "Reset password",
        token: q.token,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResetForm {
    token: String,
    new_password: String,
}

pub(crate) async fn reset_submit(
    State(st): State<AppState>,
    Form(form): Form<ResetForm>,
) -> Response {
    let req = ResetReq {
        token: form.token,
        new_password: form.new_password,
    };
    match flow::reset_password(State(st), Json(req)).await {
        Ok(_) => done(
            "Password updated. Every session was signed out.",
            "/login".to_owned(),
            "Sign in",
        ),
        Err(e) => fail(&e),
    }
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/login",
            get(auth_pages::login_page).post(auth_pages::login_submit),
        )
        .route(
            "/register",
            get(auth_pages::register_page).post(auth_pages::register_submit),
        )
        .route(
            "/verify",
            get(auth_pages::verify_page).post(auth_pages::verify_submit),
        )
        .route(
            "/forgot",
            get(auth_pages::forgot_page).post(auth_pages::forgot_submit),
        )
        .route(
            "/reset",
            get(auth_pages::reset_page).post(auth_pages::reset_submit),
        )
        .route("/logout", post(auth_pages::logout))
}
