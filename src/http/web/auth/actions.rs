//! POST handlers: thin adapters from form fields to `service::auth`.

use axum::{extract::State, http::HeaderMap, response::Response};
use secrecy::SecretString;
use serde::Deserialize;

use super::{LOGIN, session};
use crate::{
    AppState,
    auth::{ClientInfo, LoginCredentials, NewAccount},
    error::AppError,
    http::{
        cookie::SessionCookie,
        htmx::ActionError,
        web::{
            alert::Alert,
            error::WebError,
            form::{WebForm, require_htmx},
        },
    },
    service,
    util::NonEmptyString,
};

const INVALID_TOKEN: &str = "invalid or expired token";

/// Parses an e-mailed token. A malformed token gets the same message as an
/// expired one, so the response says nothing about token formats.
fn parse_token<T: TryFrom<String>>(raw: &str) -> Result<T, AppError> {
    T::try_from(raw.trim().to_owned()).map_err(|_| AppError::BadRequest(INVALID_TOKEN.into()))
}

// --------------------------------------------------------------------- login

#[derive(Debug, Deserialize)]
pub(super) struct LoginForm {
    identifier: String,
    password: SecretString,
}

pub(super) async fn login(
    State(ctx): State<AppState>,
    client: ClientInfo,
    WebForm(form): WebForm<LoginForm>,
) -> Result<Response, WebError> {
    let creds = LoginCredentials::new(&form.identifier, form.password);
    let token = service::auth::login(&ctx, &client, creds).await?;
    Ok(session::sign_in(&token)?)
}

/// Always ends with a cleared cookie and a redirect, even if the session was
/// already gone, so logout is idempotent.
pub(super) async fn logout(
    State(ctx): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ActionError> {
    require_htmx(&headers)?;
    if let Some(token) = SessionCookie::get(&headers) {
        service::auth::logout(&ctx, token).await?;
    }
    Ok(session::sign_out(LOGIN))
}

// ------------------------------------------------------------------ register

#[derive(Debug, Deserialize)]
pub(super) struct RegisterForm {
    email: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    display_name: String,
    password: SecretString,
}

pub(super) async fn register(
    State(ctx): State<AppState>,
    WebForm(form): WebForm<RegisterForm>,
) -> Result<Alert, WebError> {
    let account = NewAccount {
        email: form.email,
        username: NonEmptyString::non_empty(&form.username),
        display_name: NonEmptyString::non_empty(&form.display_name),
        password: form.password,
    };

    // `None` (e-mail already registered) is deliberately indistinguishable
    // from a fresh account: same alert, same status.
    service::auth::create_account(&ctx, account).await?;

    Ok(
        Alert::success("Account created. Check your e-mail for a verification link.")
            .link("/verify", "Enter token"),
    )
}

// -------------------------------------------------------------------- verify

#[derive(Debug, Deserialize)]
pub(super) struct VerifyForm {
    token: String,
}

pub(super) async fn verify(
    State(ctx): State<AppState>,
    WebForm(form): WebForm<VerifyForm>,
) -> Result<Alert, WebError> {
    let token = parse_token(&form.token)?;
    service::auth::verify_email(&ctx.db, token).await?;

    Ok(Alert::success("E-mail verified. You can sign in now.").link(LOGIN, "Sign in"))
}

// ------------------------------------------------------------ forgot / reset

#[derive(Debug, Deserialize)]
pub(super) struct ForgotForm {
    email: String,
}

/// Answers identically whether or not the account exists, so the response
/// can't be used to enumerate accounts.
pub(super) async fn forgot(
    State(ctx): State<AppState>,
    WebForm(form): WebForm<ForgotForm>,
) -> Result<Alert, WebError> {
    service::auth::forgot_password(&ctx, form.email).await?;

    Ok(Alert::success(
        "If that account exists, we've sent a link to reset the password.",
    ))
}

#[derive(Debug, Deserialize)]
pub(super) struct ResetForm {
    token: String,
    new_password: SecretString,
}

pub(super) async fn reset(
    State(ctx): State<AppState>,
    WebForm(form): WebForm<ResetForm>,
) -> Result<Alert, WebError> {
    let token = parse_token(&form.token)?;
    service::auth::reset_password(&ctx.db, token, form.new_password).await?;

    Ok(Alert::success("Password updated. Every session was signed out.").link(LOGIN, "Sign in"))
}
