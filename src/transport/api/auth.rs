use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, header},
    response::IntoResponse,
    routing::post,
};
use http::StatusCode;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::{
    identity::{ClientInfo, NewAccount, ResetToken, VerifyToken},
    prelude::*,
    service,
    transport::{
        cookie::SessionCookie,
        dto::auth::{LoginRequest, RegisterResponse},
        extract::body::{AppJson, JsonOrForm},
    },
};

// ---------------------------------------------------------------- register

/// Starts registration. New accounts are `pending` until they verify their
/// e-mail.
///
/// The response is identical whether the address is new or already
/// registered, so it can't be used to enumerate accounts. Nothing from the
/// created account (ids, status) may be added here.
async fn register(
    State(state): State<AppState>,
    AppJson(req): AppJson<NewAccount>,
) -> Result<(StatusCode, AppJson<RegisterResponse>)> {
    // `None` (e-mail already registered) is deliberately indistinguishable
    // from a fresh account.
    service::auth::create_account(&state, req).await?;

    Ok((StatusCode::ACCEPTED, AppJson(RegisterResponse::new())))
}

// ------------------------------------------------------------ verify e-mail

#[derive(Deserialize)]
pub(crate) struct TokenReq {
    pub(crate) token: String,
}

async fn verify_email(
    State(pool): State<SqlitePool>,
    AppJson(req): AppJson<TokenReq>,
) -> Result<Json<Value>> {
    let verify_token = VerifyToken::try_from(req.token)?;
    crate::service::auth::verify_email(&pool, verify_token).await?;
    Ok(Json(json!({ "ok": true })))
}

// -------------------------------------------------------------------- login

async fn login(
    State(state): State<AppState>,
    client: ClientInfo,
    JsonOrForm(req): JsonOrForm<LoginRequest>,
) -> Result<impl IntoResponse> {
    let token = service::auth::login(&state.db, &client, req.into()).await?;
    Ok((
        [(
            header::SET_COOKIE,
            SessionCookie::issue(&token, state.config.secure_cookie)?,
        )],
        StatusCode::NO_CONTENT,
    ))
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let maybe_token = SessionCookie::get(&headers);

    if let Some(session_cookie_token) = maybe_token {
        service::auth::logout(&state.db, session_cookie_token).await?;
    }

    Ok((
        [(
            header::SET_COOKIE,
            SessionCookie::clear(state.config.secure_cookie)?,
        )],
        StatusCode::NO_CONTENT,
    ))
}

// ----------------------------------------------------------- password reset

#[derive(Deserialize)]
pub(crate) struct ForgotReq {
    pub email: String,
}

async fn forgot_password(
    State(state): State<AppState>,
    Json(req): Json<ForgotReq>,
) -> Result<Json<Value>, AppError> {
    let email = req.email.trim().to_owned();

    service::auth::forgot_password(&state, email).await?;

    // Same answer whether or not the account exists (no user enumeration).
    Ok(Json(json!({
        "ok": true,
        "message": "If that address is registered, a reset link has been sent.",
    })))
}

#[derive(Deserialize)]
pub(crate) struct ResetReq {
    pub token: String,
    pub new_password: SecretString,
}

async fn reset_password(
    State(pool): State<SqlitePool>,
    Json(req): Json<ResetReq>,
) -> Result<StatusCode, AppError> {
    let token = ResetToken::try_from(req.token)?;

    let _throw_away = service::auth::reset_password(&pool, token, req.new_password).await;

    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        // --- public auth flow
        .route("/auth/register", post(register))
        .route("/auth/verify", post(verify_email))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/forgot", post(forgot_password))
        .route("/auth/reset", post(reset_password))
}
