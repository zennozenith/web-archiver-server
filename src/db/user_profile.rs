use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::json;
use sqlx::{FromRow, SqlitePool};

use crate::{
    crypto::sha256_hex,
    db::{audit::audit, user::UserRow},
    helper::NonEmptyString,
    prelude::*,
    time::{DAY_MS, add_ms, now_ms},
    token::ApiKeyToken,
    user_select,
};

#[derive(Debug, FromRow)]
pub(crate) struct ProfileRow {
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub birth_date: Option<String>,
    pub locale: String,
    pub timezone: String,
    pub updated_at: i64,
}

// ------------------------------------------------------------------- /me

pub(crate) async fn get_me(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<(UserRow, ProfileRow), AppError> {
    let user_row: UserRow = sqlx::query_as(concat!(user_select!(), " WHERE id = ?"))
        .bind(user_id)
        .fetch_one(pool)
        .await?;

    let profile: ProfileRow = sqlx::query_as(
        "SELECT display_name, first_name, last_name, avatar_url, bio, birth_date,
                locale, timezone, updated_at
           FROM user_profiles WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    Ok((user_row, profile))
}

#[derive(Deserialize)]
pub(crate) struct ProfileReq {
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub bio: Option<String>,
    /// YYYY-MM-DD (the schema has a CHECK for this format)
    pub birth_date: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
}

pub(crate) async fn update_profile(pool: &SqlitePool, user_id: i64, req: ProfileReq) -> Result<()> {
    // updated_at is maintained by the trg_profiles_updated_at trigger.
    sqlx::query(
        "UPDATE user_profiles
            SET display_name = ?, first_name = ?, last_name = ?, bio = ?, birth_date = ?,
                locale   = COALESCE(?, locale),
                timezone = COALESCE(?, timezone)
          WHERE user_id = ?",
    )
    .bind(req.display_name.as_deref().map(NonEmptyString::non_empty))
    .bind(req.first_name.as_deref().map(NonEmptyString::non_empty))
    .bind(req.last_name.as_deref().map(NonEmptyString::non_empty))
    .bind(req.bio.as_deref().map(NonEmptyString::non_empty))
    .bind(req.birth_date.as_deref().map(NonEmptyString::non_empty))
    .bind(req.locale.as_deref().map(NonEmptyString::non_empty))
    .bind(req.timezone.as_deref().map(NonEmptyString::non_empty))
    .bind(user_id)
    .execute(pool)
    .await?;

    audit(pool, Some(user_id), Some(user_id), "profile.updated", None).await;
    Ok(())
}

// -------------------------------------------------------------- sessions

#[derive(Debug, FromRow)]
pub(crate) struct SessionRow {
    pub public_id: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub expires_at: i64,
}

pub(crate) async fn list_sessions(pool: &SqlitePool, user_id: i64) -> Result<Vec<SessionRow>> {
    let rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT public_id, ip_address, user_agent, created_at, last_used_at, expires_at
           FROM v_active_sessions
          WHERE user_id = ?
          ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub(crate) async fn revoke_session(
    pool: &SqlitePool,
    user_id: i64,
    public_id: String,
) -> Result<()> {
    let res = sqlx::query(
        "UPDATE sessions SET revoked_at = ?, revoked_reason = 'user_revoked'
          WHERE public_id = ? AND user_id = ? AND revoked_at IS NULL",
    )
    .bind(now_ms())
    .bind(&public_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("session"));
    }
    audit(
        pool,
        Some(user_id),
        Some(user_id),
        "session.revoked",
        Some(json!({ "session": public_id })),
    )
    .await;
    Ok(())
}

// -------------------------------------------------------------- API keys

#[derive(Debug, FromRow)]
pub(crate) struct ApiKeyRow {
    pub public_id: String,
    pub name: String,
    pub key_prefix: String,
    pub scopes: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
}

pub(crate) async fn list_api_keys(pool: &SqlitePool, user_id: i64) -> Result<Vec<ApiKeyRow>> {
    let rows: Vec<ApiKeyRow> = sqlx::query_as(
        "SELECT public_id, name, key_prefix, scopes, created_at, last_used_at, expires_at
           FROM api_keys
          WHERE user_id = ? AND revoked_at IS NULL
          ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[derive(Deserialize)]
pub(crate) struct CreateKeyReq {
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_in_days: Option<i64>,
}

pub(crate) async fn create_api_key(
    pool: &SqlitePool,
    user_id: i64,
    req: CreateKeyReq,
) -> Result<(String, ApiKeyToken)> {
    let name = req.name.trim().to_owned();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }

    let key = ApiKeyToken::new()?;
    let prefix: String = key.expose_secret().chars().take(8).collect();
    let now = now_ms();
    let expires_at = req
        .expires_in_days
        .filter(|d| *d > 0)
        .map(|d| add_ms(now, d.saturating_mul(DAY_MS)));
    let scopes_json =
        serde_json::to_string(&req.scopes).map_err(|e| AppError::Internal(Box::new(e)))?;

    let public_id: String = sqlx::query_scalar(
        "INSERT INTO api_keys (user_id, name, key_prefix, key_hash, scopes, expires_at)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING public_id",
    )
    .bind(user_id)
    .bind(&name)
    .bind(&prefix)
    .bind(sha256_hex(key.expose_secret()))
    .bind(&scopes_json)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;

    audit(
        pool,
        Some(user_id),
        Some(user_id),
        "api_key.created",
        Some(json!({ "name": name })),
    )
    .await;

    // The plaintext key is shown exactly once - only its hash is stored.
    Ok((public_id, key))
}

pub(crate) async fn revoke_api_key(
    pool: &SqlitePool,
    user_id: i64,
    public_id: String,
) -> Result<()> {
    let res = sqlx::query(
        "UPDATE api_keys SET revoked_at = ?
          WHERE public_id = ? AND user_id = ? AND revoked_at IS NULL",
    )
    .bind(now_ms())
    .bind(&public_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("api_key"));
    }
    audit(
        pool,
        Some(user_id),
        Some(user_id),
        "api_key.revoked",
        Some(json!({ "key": public_id })),
    )
    .await;
    Ok(())
}
