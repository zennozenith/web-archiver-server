use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{delete, get, put},
};
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::{
    db::{
        user::UserRow,
        user_profile::{CreateKeyReq, ProfileReq, ProfileRow, SessionRow},
    },
    error::{AppError, ForbiddenReason},
    extract::auth_user::AuthUser,
    prelude::*,
    time::{ms_to_dt, opt_ms_to_dt},
};

/// Public representation: the integer `id` is never exposed, only `public_id`.
#[derive(Debug, Serialize)]
struct UserDto {
    pub public_id: String,
    pub email: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub username: Option<String>,
    pub phone_e164: Option<String>,
    pub status: String,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub roles: Vec<String>,
}

impl From<UserRow> for UserDto {
    fn from(r: UserRow) -> Self {
        Self {
            public_id: r.public_id,
            email: r.email,
            email_verified_at: opt_ms_to_dt(r.email_verified_at),
            username: r.username,
            phone_e164: r.phone_e164,
            status: r.status,
            last_login_at: opt_ms_to_dt(r.last_login_at),
            created_at: ms_to_dt(r.created_at),
            display_name: r.display_name,
            first_name: r.first_name,
            last_name: r.last_name,
            avatar_url: r.avatar_url,
            locale: r.locale,
            timezone: r.timezone,
            roles: r
                .roles
                .map_or_default(|s| s.split(',').map(str::to_owned).collect()),
        }
    }
}

#[derive(Debug, Serialize)]
struct ProfileDto {
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub birth_date: Option<String>,
    pub locale: String,
    pub timezone: String,
    pub updated_at: DateTime<Utc>,
}

impl From<ProfileRow> for ProfileDto {
    fn from(p: ProfileRow) -> Self {
        Self {
            display_name: p.display_name,
            first_name: p.first_name,
            last_name: p.last_name,
            avatar_url: p.avatar_url,
            bio: p.bio,
            birth_date: p.birth_date,
            locale: p.locale,
            timezone: p.timezone,
            updated_at: ms_to_dt(p.updated_at),
        }
    }
}
// ------------------------------------------------------------------- /me

async fn get_me(State(pool): State<SqlitePool>, user: AuthUser) -> Result<Json<Value>, AppError> {
    user.require("profile:read")?;

    let (user_row, profile_row) = crate::db::user_profile::get_me(&pool, user.id).await?;

    Ok(Json(json!({
        "user": UserDto::from(user_row),
        "profile": ProfileDto::from(profile_row),
        "permissions": &user.permissions,
    })))
}

async fn update_profile(
    State(pool): State<SqlitePool>,
    user: AuthUser,
    Json(req): Json<ProfileReq>,
) -> Result<Json<Value>, AppError> {
    user.require("profile:write")?;

    crate::db::user_profile::update_profile(&pool, user.id, req).await?;

    Ok(Json(json!({ "ok": true })))
}

// -------------------------------------------------------------- sessions

#[derive(Serialize)]
struct SessionDto {
    public_id: String,
    ip_address: Option<String>,
    user_agent: Option<String>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    current: bool,
}

async fn list_sessions(
    State(pool): State<SqlitePool>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let rows: Vec<SessionRow> = crate::db::user_profile::list_sessions(&pool, user.id).await?;

    let out: Vec<SessionDto> = rows
        .into_iter()
        .map(|r| SessionDto {
            current: user.session_public_id.as_deref() == Some(r.public_id.as_str()),
            public_id: r.public_id,
            ip_address: r.ip_address,
            user_agent: r.user_agent,
            created_at: ms_to_dt(r.created_at),
            last_used_at: opt_ms_to_dt(r.last_used_at),
            expires_at: ms_to_dt(r.expires_at),
        })
        .collect();
    Ok(Json(json!({ "sessions": out })))
}

async fn revoke_session(
    State(pool): State<SqlitePool>,
    user: AuthUser,
    Path(public_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    // TODO: check if user can revoke others session
    crate::db::user_profile::revoke_session(&pool, user.id, public_id).await?;

    Ok(Json(json!({ "ok": true })))
}

// -------------------------------------------------------------- API keys

#[derive(Serialize)]
struct ApiKeyDto {
    public_id: String,
    name: String,
    key_prefix: String,
    scopes: Vec<String>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}

async fn list_api_keys(
    State(pool): State<SqlitePool>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let rows = crate::db::user_profile::list_api_keys(&pool, user.id).await?;

    let keys: Vec<ApiKeyDto> = rows
        .into_iter()
        .map(|r| ApiKeyDto {
            public_id: r.public_id,
            name: r.name,
            key_prefix: r.key_prefix,
            scopes: serde_json::from_str(&r.scopes).unwrap_or_default(),
            created_at: ms_to_dt(r.created_at),
            last_used_at: opt_ms_to_dt(r.last_used_at),
            expires_at: opt_ms_to_dt(r.expires_at),
        })
        .collect();
    Ok(Json(json!({ "api_keys": keys })))
}

async fn create_api_key(
    State(pool): State<SqlitePool>,
    user: AuthUser,
    Json(req): Json<CreateKeyReq>,
) -> Result<Json<Value>, AppError> {
    if user.via_api_key {
        return Err(AppError::Forbidden(ForbiddenReason::ApiKeyMint)); // keys cannot mint more keys
    }
    // A key may only carry scopes its owner actually has.
    if req.scopes.iter().any(|s| !user.can(s)) {
        return Err(AppError::Forbidden(ForbiddenReason::Permission));
    }

    let (public_id, key) = crate::db::user_profile::create_api_key(&pool, user.id, req).await?;

    // The plaintext key is shown exactly once - only its hash is stored.
    Ok(Json(
        json!({ "public_id": public_id, "key": key.expose_secret() }),
    ))
}

async fn revoke_api_key(
    State(pool): State<SqlitePool>,
    user: AuthUser,
    Path(public_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    crate::db::user_profile::revoke_api_key(&pool, user.id, public_id).await?;

    Ok(Json(json!({ "ok": true })))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        // --- the signed-in user
        .route("/me", get(get_me))
        .route("/me/profile", put(update_profile))
        .route("/me/sessions", get(list_sessions))
        .route("/me/sessions/{public_id}", delete(revoke_session))
        .route("/me/api-keys", get(list_api_keys).post(create_api_key))
        .route("/me/api-keys/{public_id}", delete(revoke_api_key))
}
