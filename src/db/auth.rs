use std::collections::HashSet;

use secrecy::ExposeSecret as _;
use sqlx::{FromRow, Result, SqliteConnection, SqliteExecutor, SqlitePool};

use crate::{
    auth::{AccountStatus, ClientInfo, SESSION_TTL_MS, SessionToken, TokenPurpose, UserProfile},
    infra::sha256_hex,
    util::time::add_ms,
};

#[derive(FromRow)]
pub(crate) struct SessionHit {
    #[sqlx(flatten)]
    pub user: UserProfile,
    pub session_public_id: String,
    pub last_used_at: i64,
}

#[derive(FromRow)]
pub(crate) struct ApiKeyHit {
    #[sqlx(flatten)]
    pub user: UserProfile,
    pub key_id: i64,
    pub scopes: String, // JSON array
}

pub(crate) async fn find_session(
    pool: &SqlitePool,
    token_hash: &str,
) -> Result<Option<SessionHit>> {
    // Assumes v_active_sessions exposes last_used_at; add it to the view if not.
    sqlx::query_as(
        "SELECT u.id, u.public_id, u.email, u.username,
                s.public_id AS session_public_id, s.last_used_at
           FROM v_active_sessions s
           JOIN users u ON u.id = s.user_id
          WHERE s.token_hash = ? AND u.status = 'active' AND u.deleted_at IS NULL",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

pub(crate) async fn touch_session(pool: &SqlitePool, public_id: &str, now: i64) -> Result<()> {
    sqlx::query("UPDATE sessions SET last_used_at = ? WHERE public_id = ?")
        .bind(now)
        .bind(public_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn find_api_key(
    pool: &SqlitePool,
    key_hash: &str,
    now: i64,
) -> Result<Option<ApiKeyHit>> {
    sqlx::query_as(
        "SELECT u.id, u.public_id, u.email, u.username, k.id AS key_id, k.scopes
           FROM api_keys k
           JOIN users u ON u.id = k.user_id
          WHERE k.key_hash = ? AND k.revoked_at IS NULL
            AND (k.expires_at IS NULL OR k.expires_at > ?)
            AND u.status = 'active' AND u.deleted_at IS NULL",
    )
    .bind(key_hash)
    .bind(now)
    .fetch_optional(pool)
    .await
}

pub(crate) async fn touch_api_key(pool: &SqlitePool, key_id: i64, now: i64) -> Result<()> {
    sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?")
        .bind(now)
        .bind(key_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Roles and permissions in one round trip. Both views already drop expired grants.
pub(crate) async fn roles_and_permissions(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<(Vec<String>, HashSet<String>)> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT 'role', role FROM v_user_roles WHERE user_id = ?1
         UNION ALL
         SELECT 'perm', permission FROM v_user_permissions WHERE user_id = ?1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut roles = Vec::new();
    let mut perms = HashSet::new();
    for (kind, value) in rows {
        if kind == "role" {
            roles.push(value);
        } else {
            perms.insert(value);
        }
    }
    Ok((roles, perms))
}

// ==================================================================

/// A token row that was valid at lookup time.
pub(crate) struct ValidToken {
    pub id: i64,
    /// `None` only for `invite` tokens; every other purpose always has a user.
    pub user_id: Option<i64>,
}

/// Read-only check, e.g. to decide whether to show the "new password" form.
/// Does not consume the token.
pub(crate) async fn find_valid_token(
    conn: &mut SqliteConnection,
    purpose: TokenPurpose,
    token_hash: &str,
    now: i64,
) -> sqlx::Result<Option<ValidToken>> {
    let row: Option<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, user_id FROM auth_tokens
          WHERE token_hash = ?1 AND purpose = ?2
            AND used_at IS NULL AND expires_at > ?3",
    )
    .bind(token_hash)
    .bind(purpose)
    .bind(now)
    .fetch_optional(conn)
    .await?;

    Ok(row.map(|(id, user_id)| ValidToken { id, user_id }))
}

/// The current password plus the last five from `password_history`.
pub(crate) async fn recent_password_hashes(
    tx: &mut SqliteConnection,
    user_id: i64,
) -> Result<Vec<String>> {
    let mut previous: Vec<String> = sqlx::query_scalar(
        "SELECT password_hash FROM password_history
          WHERE user_id = ? ORDER BY created_at DESC LIMIT 5",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    let current: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM user_credentials WHERE user_id = ?")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(current) = current {
        previous.push(current);
    }
    Ok(previous)
}

pub(crate) async fn insert_password_history_from_current(
    tx: &mut SqliteConnection,
    user_id: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO password_history (user_id, password_hash)
         SELECT user_id, password_hash FROM user_credentials WHERE user_id = ?",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn upsert_password(
    tx: &mut SqliteConnection,
    user_id: i64,
    new_hash: &str,
    now: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO user_credentials (user_id, password_hash) VALUES (?1, ?2)
         ON CONFLICT(user_id) DO UPDATE SET
             password_hash       = excluded.password_hash,
             password_changed_at = ?3,
             must_change         = 0,
             failed_attempts     = 0,
             last_failed_at      = NULL,
             locked_until        = NULL",
    )
    .bind(user_id)
    .bind(new_hash)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    Ok(())
}

pub(crate) async fn consume_token(
    conn: &mut SqliteConnection,
    token_id: i64,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE auth_tokens SET used_at = ? WHERE id = ?")
        .bind(now)
        .bind(token_id)
        .execute(conn)
        .await?;
    Ok(())
}

pub(crate) async fn revoke_all_sessions(
    tx: &mut SqliteConnection,
    user_id: i64,
    revoke_reason: &'static str, // eg: "password_reset"
    now: i64,
) -> Result<()> {
    // A password reset signs the user out everywhere.
    sqlx::query(
        "UPDATE sessions SET revoked_at = ?, revoked_reason = ?
          WHERE user_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(revoke_reason)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

// ==========================================================================

/// Id of the active, non-deleted user with this e-mail.
pub(crate) async fn find_active_user_id_by_email<'e>(
    executor: impl SqliteExecutor<'e>,
    email: &str,
) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar(
        "SELECT id FROM users
          WHERE email = ? AND status = 'active' AND deleted_at IS NULL",
    )
    .bind(email)
    .fetch_optional(executor)
    .await
}

/// Marks every outstanding token of this purpose as used, so only the newest
/// link works. Returns how many were invalidated.
pub(crate) async fn invalidate_open_tokens(
    conn: &mut SqliteConnection,
    user_id: i64,
    purpose: TokenPurpose,
    now: i64,
) -> sqlx::Result<u64> {
    let res = sqlx::query(
        "UPDATE auth_tokens SET used_at = ?1
          WHERE user_id = ?2 AND purpose = ?3 AND used_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .bind(purpose)
    .execute(conn)
    .await?;
    Ok(res.rows_affected())
}

pub(crate) struct NewAuthToken<'a> {
    pub user_id: Option<i64>,
    pub purpose: TokenPurpose,
    pub token_hash: &'a str,
    pub target: Option<&'a str>,
    pub expires_at: i64,
}

pub(crate) async fn insert_token(
    conn: &mut SqliteConnection,
    t: NewAuthToken<'_>,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO auth_tokens (user_id, purpose, token_hash, target, expires_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(t.user_id)
    .bind(t.purpose)
    .bind(t.token_hash)
    .bind(t.target)
    .bind(t.expires_at)
    .execute(conn)
    .await?;
    Ok(())
}

// ==========================================================================

pub(crate) struct NewLoginAttempt<'a> {
    pub identifier: &'a str,
    pub user_id: Option<i64>,
    pub ip: &'a str,
    pub user_agent: &'a str,
    pub failure_reason: Option<&'a str>, // None = success
}

pub(crate) async fn insert_login_attempt<'e>(
    executor: impl SqliteExecutor<'e>,
    a: NewLoginAttempt<'_>,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO login_attempts
             (identifier, user_id, ip_address, user_agent, success, failure_reason)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(a.identifier)
    .bind(a.user_id)
    .bind(a.ip)
    .bind(a.user_agent)
    .bind(i64::from(a.failure_reason.is_none()))
    .bind(a.failure_reason)
    .execute(executor)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
pub(crate) struct LoginRow {
    pub id: i64,
    pub status: AccountStatus,
    pub password_hash: String,
    pub locked_until: Option<i64>,
}

pub(crate) async fn find_login_user(
    pool: &SqlitePool,
    ident: &str,
) -> Result<Option<LoginRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT u.id, u.status, c.password_hash, c.locked_until
           FROM users u
           JOIN user_credentials c ON c.user_id = u.id
          WHERE (u.email = ?1 OR u.username = ?1) AND u.deleted_at IS NULL",
    )
    .bind(ident)
    .fetch_optional(pool)
    .await
}

pub(crate) async fn record_bad_password(
    pool: &SqlitePool,
    user_id: i64,
    now: i64,
    max_failed_attempts: i64,
    lockout_ms: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_credentials
            SET failed_attempts = failed_attempts + 1,
                last_failed_at  = ?1,
                locked_until    = CASE WHEN failed_attempts + 1 >= ?2 THEN ?3 ELSE locked_until END
          WHERE user_id = ?4",
    )
    .bind(now)
    .bind(max_failed_attempts)
    .bind(add_ms(now, lockout_ms))
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn create_session(
    pool: &SqlitePool,
    user_id: i64,
    client_info: &ClientInfo,
    token_id: &str,
    session_token_hash: &str,
    now: i64,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE user_credentials SET failed_attempts = 0, last_failed_at = NULL, locked_until = NULL
          WHERE user_id = ?",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE users SET last_login_at = ?, last_login_ip = ?, login_count = login_count + 1
          WHERE id = ?",
    )
    .bind(now)
    .bind(client_info.ip.to_string())
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, family_id, ip_address, user_agent, last_used_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(session_token_hash)
    .bind(token_id)
    .bind(client_info.ip.to_string())
    .bind(&client_info.user_agent)
    .bind(now)
    .bind(add_ms(now, SESSION_TTL_MS))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn delete_session(
    pool: &SqlitePool,
    session_token: SessionToken,
    now: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE sessions SET revoked_at = ?, revoked_reason = 'logout'
              WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(sha256_hex(session_token.expose_secret()))
    .execute(pool)
    .await?;
    Ok(())
}

// ==========================================================================

/// Inserts a `pending` user. The `trg_users_create_profile` trigger also
/// creates the empty profile row.
pub(crate) async fn insert_pending_user(
    conn: &mut SqliteConnection,
    email: &str,
    username: Option<&str>,
    now: i64,
    terms_version: &str,
) -> sqlx::Result<(i64, String)> {
    sqlx::query_as(
        "INSERT INTO users (email, username, status, email_verified_at, terms_accepted_at, terms_version)
         VALUES (?, ?, 'pending', NULL, ?, ?)
         RETURNING id, public_id",
    )
    .bind(email)
    .bind(username)
    .bind(now)
    .bind(terms_version)
    .fetch_one(conn)
    .await
}

pub(crate) async fn set_display_name(
    conn: &mut SqliteConnection,
    user_id: i64,
    display_name: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE user_profiles SET display_name = ? WHERE user_id = ?")
        .bind(display_name)
        .bind(user_id)
        .execute(conn)
        .await?;
    Ok(())
}

pub(crate) async fn insert_credentials(
    conn: &mut SqliteConnection,
    user_id: i64,
    password_hash: &str,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO user_credentials (user_id, password_hash) VALUES (?, ?)")
        .bind(user_id)
        .bind(password_hash)
        .execute(conn)
        .await?;
    Ok(())
}

/// Returns `false` if no role with that name exists.
pub(crate) async fn grant_role(
    conn: &mut SqliteConnection,
    user_id: i64,
    role: &str,
) -> sqlx::Result<bool> {
    let res = sqlx::query(
        "INSERT INTO user_roles (user_id, role_id) SELECT ?, id FROM roles WHERE name = ?",
    )
    .bind(user_id)
    .bind(role)
    .execute(conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Marks the e-mail verified and promotes `pending` to `active`.
/// Other statuses (suspended, deactivated) are left alone.
/// Returns `false` if the user doesn't exist or is deleted.
pub(crate) async fn mark_email_verified(
    conn: &mut SqliteConnection,
    user_id: i64,
    now: i64,
) -> sqlx::Result<bool> {
    let res = sqlx::query(
        "UPDATE users
            SET email_verified_at = ?1,
                status = CASE WHEN status = 'pending' THEN 'active' ELSE status END
          WHERE id = ?2 AND deleted_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Atomically marks a valid token as used and returns it. `None` if the token
/// is unknown, expired, already used, or issued for a different purpose.
///
/// One statement, so two concurrent requests can't both succeed. Called inside
/// the service's transaction, a later failure rolls `used_at` back and the
/// user can retry with the same link.
pub(crate) async fn take_valid_token(
    conn: &mut SqliteConnection,
    purpose: TokenPurpose,
    token_hash: &str,
    now: i64,
) -> sqlx::Result<Option<ValidToken>> {
    let row: Option<(i64, Option<i64>)> = sqlx::query_as(
        "UPDATE auth_tokens SET used_at = ?3
          WHERE token_hash = ?1 AND purpose = ?2
            AND used_at IS NULL AND expires_at > ?3
          RETURNING id, user_id",
    )
    .bind(token_hash)
    .bind(purpose)
    .bind(now)
    .fetch_optional(conn)
    .await?;

    Ok(row.map(|(id, user_id)| ValidToken { id, user_id }))
}
