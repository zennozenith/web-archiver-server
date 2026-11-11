use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{ConnectInfo, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
    routing::post,
};
use http::StatusCode;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::{
    db::auth::{AccountStatus, LoginReq, NewAccount, create_account},
    error::AppError,
    extract::{
        ApiJson,
        auth_user::{SessionCookie, cookie_value},
    },
    prelude::*,
    token::{ResetToken, SessionToken, VerifyToken},
};

// ---------------------------------------------------------------- register

/// Creates user + credentials + role in one transaction. New accounts are
/// `pending` until they verify their e-mail.
async fn register(
    State(pool): State<SqlitePool>,
    ApiJson(req): ApiJson<NewAccount>,
) -> Result<(StatusCode, Json<Value>)> {
    let created = create_account(pool, req).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "public_id": created.public_id,
            "status": AccountStatus::Pending,
            "message": "Account created. Verify your e-mail before logging in.",
        })),
    ))
}

// ------------------------------------------------------------ verify e-mail

#[derive(Deserialize)]
pub(crate) struct TokenReq {
    pub(crate) token: String,
}

async fn verify_email(
    State(pool): State<SqlitePool>,
    ApiJson(req): ApiJson<TokenReq>,
) -> Result<Json<Value>> {
    let verify_token = VerifyToken::try_from(req.token)?;
    crate::db::auth::verify_email(pool, verify_token).await?;
    Ok(Json(json!({ "ok": true })))
}

// -------------------------------------------------------------------- login

async fn login(
    State(pool): State<SqlitePool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(req): ApiJson<LoginReq>,
) -> Result<Response, AppError> {
    let ip = addr.ip().to_string();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    let session_cookie_token = crate::db::auth::login(&pool, &ip, &user_agent, req).await?;

    Ok((
        [(
            header::SET_COOKIE,
            SessionCookie::cookie(&session_cookie_token),
        )],
        Json(json!({ "ok": true })),
    )
        .into_response())
}

async fn logout(State(pool): State<SqlitePool>, headers: HeaderMap) -> Result<Response, AppError> {
    let maybe_token =
        cookie_value(&headers, SessionCookie::KEY).and_then(|v| SessionToken::try_from(v).ok());
    if let Some(session_cookie_token) = maybe_token {
        crate::db::auth::logout(&pool, session_cookie_token).await?;
    }

    Ok((
        [(header::SET_COOKIE, SessionCookie::clear())],
        Json(json!({ "ok": true })),
    )
        .into_response())
}

// ----------------------------------------------------------- password reset

#[derive(Deserialize)]
pub(crate) struct ForgotReq {
    pub email: String,
}

async fn forgot_password(
    State(pool): State<SqlitePool>,
    Json(req): Json<ForgotReq>,
) -> Result<Json<Value>, AppError> {
    let email = req.email.trim().to_owned();

    crate::db::auth::forgot_password(&pool, email).await?;
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
) -> Result<Json<Value>, AppError> {
    let token = ResetToken::try_from(req.token)?;

    let _throw_away = crate::db::auth::reset_password(&pool, token, req.new_password).await;
    Ok(Json(json!({ "ok": true })))
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
