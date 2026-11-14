use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use sqlx::SqlitePool;
use tokio::sync::OnceCell;

use crate::{
    db::{
        self,
        audit::{AuditAction, audit},
    },
    error::ForbiddenReason,
    identity::{AccountStatus, ClientInfo, LoginCredentials, SessionToken, lockout},
    infra::{hash_password, random_id, sha256_hex, verify_password},
    prelude::*,
    service::auth::{LoginFailure, attempt::AttemptLog},
    util::time::now_ms,
};

static DECOY_HASH: OnceCell<String> = OnceCell::const_new();

// Hash of a random password nobody knows, made with the live Argon2 params.
pub(super) async fn decoy_hash() -> Result<&'static str, AppError> {
    DECOY_HASH
        .get_or_try_init(|| async {
            let pw = SecretString::from(random_id()?);
            hash_password(pw).await
        })
        .await
        .map(String::as_str)
}

/// Spend the same time as a real verification, and discard the result.
async fn burn_verify(password: SecretString) {
    if let Ok(hash) = decoy_hash().await {
        let _ = verify_password(password, hash.to_owned()).await;
    }
}

pub(crate) async fn login(
    pool: &SqlitePool,
    client: &ClientInfo,
    creds: LoginCredentials,
) -> Result<SessionToken> {
    let now = now_ms();
    let identifier = creds.identifier().to_owned();
    let password = creds.into_password();

    let attempt = AttemptLog::new(pool, &identifier, client);

    let Some(user) = db::auth::find_login_user(pool, &identifier).await? else {
        burn_verify(password.clone()).await;
        return Err(attempt
            .failed(
                None,
                LoginFailure::UnknownUser,
                AppError::InvalidCredentials,
            )
            .await);
    };

    if user.locked_until.is_some_and(|t| t > now) {
        // Run the same expensive verification as a normal failed login, then
        // return the generic error so the response does not reveal whether the
        // account exists or is locked.
        burn_verify(password).await;
        return Err(attempt
            .failed(
                Some(user.id),
                LoginFailure::Locked,
                AppError::InvalidCredentials,
            )
            .await);
    }
    if !verify_password(password, user.password_hash).await {
        db::auth::record_bad_password(
            pool,
            user.id,
            now,
            lockout::MAX_FAILED_ATTEMPTS,
            lockout::LOCKOUT,
        )
        .await?;
        return Err(attempt
            .failed(
                Some(user.id),
                LoginFailure::BadPassword,
                AppError::InvalidCredentials,
            )
            .await);
    }
    if user.status != AccountStatus::Active {
        return Err(attempt
            .failed(
                Some(user.id),
                LoginFailure::Inactive,
                ForbiddenReason::from(user.status).into(),
            )
            .await);
    }

    issue_session(pool, client, user.id, now, &attempt).await
}

/// Creates the session row, records the successful attempt, and audits the login.
async fn issue_session(
    pool: &SqlitePool,
    client: &ClientInfo,
    user_id: i64,
    now: i64,
    attempt: &AttemptLog<'_>,
) -> Result<SessionToken> {
    let token = SessionToken::new()?;
    let token_hash = sha256_hex(token.expose_secret());
    let token_id = random_id()?;

    db::auth::create_session(pool, user_id, client, &token_id, &token_hash, now).await?;

    attempt.succeeded(user_id).await;
    audit(
        pool,
        Some(user_id),
        Some(user_id),
        AuditAction::AuthLogin,
        Some(json!({ "ip": client.ip, "user_agent": &client.user_agent })),
    )
    .await;

    Ok(token)
}

pub(crate) async fn logout(pool: &SqlitePool, session_token: SessionToken) -> Result<()> {
    let now = now_ms();

    db::auth::delete_session(pool, session_token, now)
        .await
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    #[tokio::test]
    async fn repeated_failures_lock_the_account() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        db::migrate(&pool).await.expect("migrations");

        let password = "Correct-Horse-9!";
        let hash = hash_password(SecretString::from(password.to_owned()))
            .await
            .expect("hash password");
        let now = now_ms();
        let user_id: i64 = sqlx::query_scalar(
            "INSERT INTO users (email, email_verified_at, status)
             VALUES ('lock@example.com', ?, 'active') RETURNING id",
        )
        .bind(now)
        .fetch_one(&pool)
        .await
        .expect("insert user");
        sqlx::query("INSERT INTO user_credentials (user_id, password_hash) VALUES (?, ?)")
            .bind(user_id)
            .bind(hash)
            .execute(&pool)
            .await
            .expect("insert credentials");

        let client = ClientInfo::new("127.0.0.1".parse().expect("ip"), Some("test"));
        for _ in 0..lockout::MAX_FAILED_ATTEMPTS {
            let result = login(
                &pool,
                &client,
                LoginCredentials::new(
                    "lock@example.com",
                    SecretString::from("wrong-password".to_owned()),
                ),
            )
            .await;
            assert!(
                matches!(result, Err(AppError::InvalidCredentials)),
                "wrong password is rejected"
            );
        }

        let result = login(
            &pool,
            &client,
            LoginCredentials::new("lock@example.com", SecretString::from(password.to_owned())),
        )
        .await;
        assert!(
            matches!(result, Err(AppError::InvalidCredentials)),
            "a locked account rejects the correct password too"
        );

        let reason: Option<String> = sqlx::query_scalar(
            "SELECT failure_reason FROM login_attempts ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("last attempt");
        assert_eq!(reason.as_deref(), Some(LoginFailure::Locked.as_str()));
    }
}
