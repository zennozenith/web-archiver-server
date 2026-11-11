use axum::extract::FromRequestParts;
use http::{HeaderMap, header, request::Parts};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use sqlx::{SqlitePool, prelude::FromRow};

use crate::{
    AppState,
    crypto::sha256_hex,
    error::{AppError, ForbiddenReason},
    time::{DAY_MS, now_ms},
};

/// The authenticated caller, resolved from either a session cookie or an
/// `Authorization: Bearer <api key>` header.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AuthUser {
    pub id: i64,
    pub public_id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
    /// Set when authenticated via cookie session (used to flag "current session").
    pub session_public_id: Option<String>,
    pub via_api_key: bool,
}

impl AuthUser {
    pub(crate) fn can(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == "*" || p == permission)
    }

    pub(crate) fn require(&self, permission: &str) -> Result<(), AppError> {
        if self.can(permission) {
            Ok(())
        } else {
            Err(AppError::Forbidden(ForbiddenReason::Permission))
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let pool = &state.db;

        if let Some(value) = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            && let Some(key) = value.strip_prefix("Bearer ")
        {
            return auth_by_api_key(pool, key.trim()).await;
        }

        let token =
            cookie_value(&parts.headers, SessionCookie::KEY).ok_or(AppError::Unauthorized)?;
        auth_by_session(pool, &token).await
    }
}

async fn auth_by_session(pool: &SqlitePool, token: &str) -> Result<AuthUser, AppError> {
    #[derive(FromRow)]
    struct User {
        id: i64,
        public_id: String,
        email: Option<String>,
        username: Option<String>,
        session_public_id: String,
    }

    let row: Option<User> = sqlx::query_as(
        "SELECT u.id, u.public_id, u.email, u.username, s.public_id as session_public_id
           FROM v_active_sessions s
           JOIN users u ON u.id = s.user_id
          WHERE s.token_hash = ?
            AND u.status = 'active'
            AND u.deleted_at IS NULL",
    )
    .bind(sha256_hex(token))
    .fetch_optional(pool)
    .await?;

    let User {
        id,
        public_id,
        email,
        username,
        session_public_id,
    } = row.ok_or(AppError::Unauthorized)?;

    sqlx::query("UPDATE sessions SET last_used_at = ? WHERE public_id = ?")
        .bind(now_ms())
        .bind(&session_public_id)
        .execute(pool)
        .await?;

    let (roles, permissions) = load_roles_and_permissions(pool, id).await?;
    Ok(AuthUser {
        id,
        public_id,
        email,
        username,
        roles,
        permissions,
        session_public_id: Some(session_public_id),
        via_api_key: false,
    })
}

async fn auth_by_api_key(pool: &SqlitePool, key: &str) -> Result<AuthUser, AppError> {
    #[derive(FromRow)]
    struct User {
        id: i64,
        public_id: String,
        email: Option<String>,
        username: Option<String>,
        key_id: i64,
        scopes_json: String,
    }

    let now = now_ms();
    let row: Option<User> = sqlx::query_as(
        "SELECT u.id, u.public_id, u.email, u.username, k.id as key_id, k.scopes
           FROM api_keys k
           JOIN users u ON u.id = k.user_id
          WHERE k.key_hash = ?
            AND k.revoked_at IS NULL
            AND (k.expires_at IS NULL OR k.expires_at > ?)
            AND u.status = 'active'
            AND u.deleted_at IS NULL",
    )
    .bind(sha256_hex(key))
    .bind(now)
    .fetch_optional(pool)
    .await?;

    let User {
        id,
        public_id,
        email,
        username,
        key_id,
        scopes_json,
    } = row.ok_or(AppError::Unauthorized)?;

    sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?")
        .bind(now)
        .bind(key_id)
        .execute(pool)
        .await?;

    let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();
    let (roles, user_perms) = load_roles_and_permissions(pool, id).await?;

    // An API key can never exceed its owner's permissions, and is further
    // limited to its own scopes.
    let permissions = if user_perms.iter().any(|p| p == "*") {
        scopes
    } else {
        user_perms
            .into_iter()
            .filter(|p| scopes.contains(p))
            .collect()
    };

    Ok(AuthUser {
        id,
        public_id,
        email,
        username,
        roles,
        permissions,
        session_public_id: None,
        via_api_key: true,
    })
}

async fn load_roles_and_permissions(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<(Vec<String>, Vec<String>), sqlx::Error> {
    // Both views already filter out expired role grants.
    let roles = sqlx::query_scalar::<_, String>("SELECT role FROM v_user_roles WHERE user_id = ?")
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    let perms = sqlx::query_scalar::<_, String>(
        "SELECT permission FROM v_user_permissions WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok((roles, perms))
}

pub(crate) struct SessionCookie;

impl SessionCookie {
    pub(crate) const KEY: &str = "sid";
    pub(crate) const TTL_MS: i64 = 7 * DAY_MS;

    pub(crate) fn cookie(token: &SecretString) -> String {
        // Add `; Secure` when serving over HTTPS.
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
            Self::KEY,
            token.expose_secret(),
            Self::TTL_MS / 1000
        )
    }

    pub(crate) fn clear() -> String {
        format!("{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0", Self::KEY)
    }
}

pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .find_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            (k == name).then(|| v.to_owned())
        })
}
