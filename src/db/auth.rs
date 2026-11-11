use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{SqliteConnection, SqliteExecutor, SqlitePool};

use crate::{
    crypto::{hash_password, random_id, sha256_hex, verify_password},
    db::audit::audit,
    error::ForbiddenReason,
    extract::auth_user::SessionCookie,
    helper::NonEmptyString,
    prelude::*,
    time::{DAY_MS, HOUR_MS, add_ms, now_ms},
    token::{ResetToken, SessionToken, VerifyToken},
};

const MAX_FAILED_ATTEMPTS: i64 = 5;
const LOCKOUT_MS: i64 = 15 * 60 * 1000;

#[derive(Deserialize)]
pub(crate) struct NewAccount {
    pub email: String,
    pub username: Option<String>,
    pub password: SecretString,
    pub display_name: Option<String>,
}

#[expect(dead_code, reason = "Might be needed later")]
pub(crate) struct CreatedAccount {
    pub user_id: i64,
    pub public_id: String,
    pub verify_token: VerifyToken,
}

#[derive(Debug, strum::Display, strum::EnumString, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub(crate) enum AccountStatus {
    Pending,
    Active,
    Suspended,
    Deactivated,
}

/// Creates user + credentials + role in one transaction. New accounts are
/// `pending` until they verify their e-mail.
pub(crate) async fn create_account(
    pool: SqlitePool,
    account: NewAccount,
) -> Result<CreatedAccount> {
    let email = NonEmptyString::trimmed(account.email)
        .map_err(|_| AppError::BadRequest("email is required".into()))?;

    check_password_len(&account.password)?;
    let username = account
        .username
        .as_deref()
        .and_then(NonEmptyString::non_empty);
    let display_name = account
        .display_name
        .as_deref()
        .and_then(NonEmptyString::non_empty);
    let password_hash = hash_password(account.password).await?;

    let now = now_ms();

    let mut tx = pool.begin().await?;

    // `users` -> trigger trg_users_create_profile also inserts the empty profile row.
    let (user_id, public_id): (i64, String) = sqlx::query_as(
        "INSERT INTO users (email, username, status, email_verified_at, terms_accepted_at, terms_version)
         VALUES (?, ?, 'pending', NULL, ?, ?)
         RETURNING id, public_id",
    )
    .bind(&email)
    .bind(username)
    .bind(now)
    .bind("2026-01")
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query("UPDATE user_profiles SET display_name = ? WHERE user_id = ?")
        .bind(display_name)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO user_credentials (user_id, password_hash) VALUES (?, ?)")
        .bind(user_id)
        .bind(password_hash)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO user_roles (user_id, role_id) SELECT ?, id FROM roles WHERE name = ?")
        .bind(user_id)
        .bind("user")
        .execute(&mut *tx)
        .await?;

    let verify_token = issue_verify_token(&mut *tx, user_id, &email, now).await?;

    tx.commit().await?;

    audit(&pool, Some(user_id), Some(user_id), "user.created", None).await;

    Ok(CreatedAccount {
        user_id,
        public_id,
        verify_token,
    })
}

pub(crate) async fn verify_email(pool: SqlitePool, token: VerifyToken) -> Result<()> {
    let now = now_ms();
    let mut tx = pool.begin().await?;

    let tok: Option<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, user_id FROM auth_tokens
          WHERE token_hash = ? AND purpose = 'verify_email'
            AND used_at IS NULL AND expires_at > ?",
    )
    .bind(sha256_hex(token.expose_secret().trim()))
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((token_id, Some(user_id))) = tok else {
        return Err(AppError::BadRequest("invalid or expired token".into()));
    };

    sqlx::query("UPDATE auth_tokens SET used_at = ? WHERE id = ?")
        .bind(now)
        .bind(token_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "UPDATE users
            SET email_verified_at = ?,
                status = CASE WHEN status = 'pending' THEN 'active' ELSE status END
          WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    audit(&pool, Some(user_id), Some(user_id), "email.verified", None).await;
    Ok(())
}

#[derive(Deserialize)]
pub(crate) struct LoginReq {
    pub(crate) identifier: String,
    pub(crate) password: SecretString,
}

/// Returns session cookie
pub(crate) async fn login(
    pool: &SqlitePool,
    ip: &str,
    user_agent: &str,
    req: LoginReq,
) -> Result<SessionToken> {
    let ident = req.identifier.trim().to_owned();
    let now = now_ms();
    let ctx = LoginContext {
        pool,
        identifier: &ident,
        ip,
        user_agent,
    };

    let Some(user) = find_login_user(pool, &ident).await? else {
        return Err(ctx
            .failure(None, "unknown_user", AppError::InvalidCredentials)
            .await);
    };

    if user.locked_until.is_some_and(|t| t > now) {
        let error =
            AppError::TooManyRequest("too many failed attempts; account temporarily locked");
        return Err(ctx.failure(Some(user.id), "locked", error).await);
    }

    if !verify_password(req.password, user.password_hash).await {
        record_bad_password(pool, user.id, now).await?;
        return Err(ctx
            .failure(Some(user.id), "bad_password", AppError::InvalidCredentials)
            .await);
    }

    if user.status != "active" {
        let error = inactive_error(&user.status);
        return Err(ctx.failure(Some(user.id), "inactive", error).await);
    }

    let token = create_session(pool, user.id, ip, user_agent, now).await?;
    ctx.success(Some(user.id)).await;
    audit(
        pool,
        Some(user.id),
        Some(user.id),
        "auth.login",
        Some(json!({ "ip": ip })),
    )
    .await;

    Ok(token)
}

pub(crate) async fn logout(pool: &SqlitePool, session_cookie_token: SessionToken) -> Result<()> {
    sqlx::query(
        "UPDATE sessions SET revoked_at = ?, revoked_reason = 'logout'
              WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(now_ms())
    .bind(sha256_hex(session_cookie_token.expose_secret()))
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn forgot_password(pool: &SqlitePool, email: String) -> Result<()> {
    let user_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM users WHERE email = ? AND status = 'active' AND deleted_at IS NULL",
    )
    .bind(&email)
    .fetch_optional(pool)
    .await?;

    if let Some(user_id) = user_id {
        let token = ResetToken::new()?;
        sqlx::query(
            "INSERT INTO auth_tokens (user_id, purpose, token_hash, target, expires_at)
             VALUES (?, 'reset_password', ?, ?, ?)",
        )
        .bind(user_id)
        .bind(sha256_hex(token.expose_secret()))
        .bind(&email)
        .bind(add_ms(now_ms(), HOUR_MS))
        .execute(pool)
        .await?;

        #[cfg(debug_assertions)]
        tracing::info!(
            "[demo e-mail] password reset token for {email}: {}",
            token.expose_secret()
        );

        audit(
            pool,
            Some(user_id),
            Some(user_id),
            "password.reset_requested",
            None,
        )
        .await;
    }

    Ok(())
}

pub(crate) async fn reset_password(
    pool: &SqlitePool,
    token: ResetToken,
    new_password: SecretString,
) -> Result<(), AppError> {
    check_password_len(&new_password)?;
    let now = now_ms();
    let mut tx = pool.begin().await?;

    let (token_id, user_id) = reset_token(&mut tx, token, now).await?;
    if password_was_used(recent_passwords(&mut tx, user_id).await?, &new_password).await {
        return Err(AppError::BadRequest(
            "choose a password you have not used recently".into(),
        ));
    }

    let new_hash = hash_password(new_password).await?;
    apply_password_change(&mut tx, user_id, token_id, &new_hash, now).await?;
    tx.commit().await?;

    audit(
        pool,
        Some(user_id),
        Some(user_id),
        "password.changed",
        Some(json!({ "via": "reset" })),
    )
    .await;
    Ok(())
}

async fn issue_verify_token<'e>(
    executor: impl SqliteExecutor<'e>,
    user_id: i64,
    email: &str,
    now: i64,
) -> Result<VerifyToken, AppError> {
    let token = VerifyToken::new()?;
    sqlx::query(
        "INSERT INTO auth_tokens (user_id, purpose, token_hash, target, expires_at)
         VALUES (?, 'verify_email', ?, ?, ?)",
    )
    .bind(user_id)
    .bind(sha256_hex(token.expose_secret()))
    .bind(email)
    .bind(add_ms(now, DAY_MS))
    .execute(executor)
    .await?;

    #[cfg(debug_assertions)]
    tracing::info!(
        "[demo e-mail] verification token for {email}: {}",
        token.expose_secret()
    );

    Ok(token)
}

fn check_password_len(password: &SecretString) -> Result<()> {
    if password.expose_secret().len() < 8 || password.expose_secret().len() > 128 {
        return Err(AppError::BadRequest(
            "password must be between 8 and 128 characters".into(),
        ));
    }
    Ok(())
}

struct LoginAttempt<'a> {
    identifier: &'a str,
    user_id: Option<i64>,
    ip: &'a str,
    user_agent: &'a str,
    success: bool,
    reason: Option<&'a str>,
}

impl<'a> LoginAttempt<'a> {
    const fn failure(
        identifier: &'a str,
        user_id: Option<i64>,
        ip: &'a str,
        user_agent: &'a str,
        reason: &'a str,
    ) -> Self {
        Self {
            identifier,
            user_id,
            ip,
            user_agent,
            success: false,
            reason: Some(reason),
        }
    }

    const fn success(
        identifier: &'a str,
        user_id: Option<i64>,
        ip: &'a str,
        user_agent: &'a str,
    ) -> Self {
        Self {
            identifier,
            user_id,
            ip,
            user_agent,
            success: true,
            reason: None,
        }
    }
}

async fn log_attempt(pool: &SqlitePool, attempt: LoginAttempt<'_>) {
    let res = sqlx::query(
        "INSERT INTO login_attempts (identifier, user_id, ip_address, user_agent, success, failure_reason)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(attempt.identifier)
    .bind(attempt.user_id)
    .bind(attempt.ip)
    .bind(attempt.user_agent)
    .bind(i64::from(attempt.success))
    .bind(attempt.reason)
    .execute(pool)
    .await;
    if let Err(e) = res {
        tracing::warn!("failed to record login attempt: {e}");
    }
}

struct LoginContext<'a> {
    pool: &'a SqlitePool,
    identifier: &'a str,
    ip: &'a str,
    user_agent: &'a str,
}

impl LoginContext<'_> {
    async fn failure(&self, user_id: Option<i64>, reason: &str, error: AppError) -> AppError {
        log_attempt(
            self.pool,
            LoginAttempt::failure(self.identifier, user_id, self.ip, self.user_agent, reason),
        )
        .await;
        error
    }

    async fn success(&self, user_id: Option<i64>) {
        log_attempt(
            self.pool,
            LoginAttempt::success(self.identifier, user_id, self.ip, self.user_agent),
        )
        .await;
    }
}

#[derive(sqlx::FromRow)]
struct LoginRow {
    id: i64,
    status: String,
    password_hash: String,
    locked_until: Option<i64>,
}

async fn find_login_user(pool: &SqlitePool, ident: &str) -> Result<Option<LoginRow>, sqlx::Error> {
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

async fn record_bad_password(pool: &SqlitePool, user_id: i64, now: i64) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE user_credentials
            SET failed_attempts = failed_attempts + 1,
                last_failed_at  = ?1,
                locked_until    = CASE WHEN failed_attempts + 1 >= ?2 THEN ?3 ELSE locked_until END
          WHERE user_id = ?4",
    )
    .bind(now)
    .bind(MAX_FAILED_ATTEMPTS)
    .bind(add_ms(now, LOCKOUT_MS))
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn create_session(
    pool: &SqlitePool,
    user_id: i64,
    ip: &str,
    user_agent: &str,
    now: i64,
) -> Result<SessionToken, AppError> {
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
    .bind(ip)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    let token = SessionToken::new()?;
    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, family_id, ip_address, user_agent, last_used_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(sha256_hex(token.expose_secret()))
    .bind(random_id()?)
    .bind(ip)
    .bind(user_agent)
    .bind(now)
    .bind(add_ms(now, SessionCookie::TTL_MS))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(token)
}

fn inactive_error(status: &str) -> AppError {
    let reason = match status {
        "pending" => ForbiddenReason::Pending,
        "suspended" => ForbiddenReason::Suspended,
        "deactivated" => ForbiddenReason::Deactivated,
        _ => ForbiddenReason::Unknown,
    };
    AppError::Forbidden(reason)
}

async fn reset_token(
    tx: &mut SqliteConnection,
    token: ResetToken,
    now: i64,
) -> Result<(i64, i64), AppError> {
    let tok: Option<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, user_id FROM auth_tokens
          WHERE token_hash = ? AND purpose = 'reset_password'
            AND used_at IS NULL AND expires_at > ?",
    )
    .bind(sha256_hex(token.expose_secret().trim()))
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((token_id, Some(user_id))) = tok else {
        return Err(AppError::BadRequest("invalid or expired token".into()));
    };
    Ok((token_id, user_id))
}

/// The current password plus the last five from `password_history`.
async fn recent_passwords(
    tx: &mut SqliteConnection,
    user_id: i64,
) -> Result<Vec<String>, AppError> {
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

async fn password_was_used(previous: Vec<String>, candidate: &SecretString) -> bool {
    // TODO: optimize
    for hash in previous {
        if verify_password(candidate.clone(), hash).await {
            return true;
        }
    }
    false
}

async fn apply_password_change(
    tx: &mut SqliteConnection,
    user_id: i64,
    token_id: i64,
    new_hash: &str,
    now: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO password_history (user_id, password_hash)
         SELECT user_id, password_hash FROM user_credentials WHERE user_id = ?",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

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

    sqlx::query("UPDATE auth_tokens SET used_at = ? WHERE id = ?")
        .bind(now)
        .bind(token_id)
        .execute(&mut *tx)
        .await?;

    // A password reset signs the user out everywhere.
    sqlx::query(
        "UPDATE sessions SET revoked_at = ?, revoked_reason = 'password_reset'
          WHERE user_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
