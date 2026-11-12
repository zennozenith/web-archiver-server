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
    auth::{AccountStatus, ClientInfo, NewAccount, ResetToken, VerifyToken},
    http::{
        cookie::SessionCookie,
        dto::auth::LoginRequest,
        extract::body::{AppJson, JsonOrForm},
    },
    prelude::*,
    service,
};

// ---------------------------------------------------------------- register

/// Creates user + credentials + role in one transaction. New accounts are
/// `pending` until they verify their e-mail.
async fn register(
    State(ctx): State<AppState>,
    AppJson(req): AppJson<NewAccount>,
) -> Result<(StatusCode, Json<Value>)> {
    let created = service::auth::create_account(&ctx, req).await?;

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
    AppJson(req): AppJson<TokenReq>,
) -> Result<Json<Value>> {
    let verify_token = VerifyToken::try_from(req.token)?;
    crate::service::auth::verify_email(&pool, verify_token).await?;
    Ok(Json(json!({ "ok": true })))
}

// -------------------------------------------------------------------- login

#[derive(Deserialize)]
pub(crate) struct LoginReq {
    pub identifier: String,
    pub password: SecretString,
}

async fn login(
    State(ctx): State<AppState>,
    client: ClientInfo,
    // ConnectInfo(addr): ConnectInfo<SocketAddr>,
    JsonOrForm(req): JsonOrForm<LoginRequest>,
) -> Result<impl IntoResponse> {
    // let ip = addr.ip().to_string();

    let token = service::auth::login(&ctx, &client, req.into()).await?;
    Ok((
        [(header::SET_COOKIE, SessionCookie::set(&token))],
        StatusCode::NO_CONTENT,
    ))
}

async fn logout(
    State(ctx): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let maybe_token = SessionCookie::get(&headers);

    if let Some(session_cookie_token) = maybe_token {
        service::auth::logout(&ctx, session_cookie_token).await?;
    }

    Ok((
        [(header::SET_COOKIE, SessionCookie::clear())],
        StatusCode::NO_CONTENT,
    ))
}

// ----------------------------------------------------------- password reset

#[derive(Deserialize)]
pub(crate) struct ForgotReq {
    pub email: String,
}

async fn forgot_password(
    State(ctx): State<AppState>,
    Json(req): Json<ForgotReq>,
) -> Result<Json<Value>, AppError> {
    let email = req.email.trim().to_owned();

    service::auth::forgot_password(&ctx, email).await?;

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
