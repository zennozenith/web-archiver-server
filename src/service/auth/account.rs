use secrecy::ExposeSecret as _;
use sqlx::{SqliteConnection, SqlitePool};

use crate::{
    app::AppState,
    auth::{
        CreatedAccount, NewAccount, TokenPurpose, VerifyToken,
        policy::{DEFAULT_ROLE, TERMS_VERSION},
    },
    db::{self, audit::audit, auth::NewAuthToken},
    error::{AppError, Result},
    infra::{hash_password, sha256_hex},
    util::time::{DAY_MS, add_ms, now_ms},
};

/// Creates user, profile name, credentials, role and verification token in one
/// transaction. New accounts are `pending` until they verify their e-mail.
pub(crate) async fn create_account(ctx: &AppState, account: NewAccount) -> Result<CreatedAccount> {
    // Hash before opening the transaction: Argon2 is slow and must not hold
    // SQLite's write lock.
    let password_hash = hash_password(account.password).await?;
    let now = now_ms();

    let mut tx = ctx.db.begin().await?;

    let (user_id, public_id) = db::auth::insert_pending_user(
        &mut tx,
        &account.email,
        account.username.as_deref(),
        now,
        TERMS_VERSION,
    )
    .await?;

    db::auth::set_display_name(&mut tx, user_id, account.display_name.as_deref()).await?;
    db::auth::insert_credentials(&mut tx, user_id, &password_hash).await?;

    if !db::auth::grant_role(&mut tx, user_id, DEFAULT_ROLE).await? {
        // Misconfigured seed data. Roll back instead of creating a user with no permissions.
        return Err(AppError::Internal(
            format!("role `{DEFAULT_ROLE}` is missing").into(),
        ));
    }

    let verify_token = issue_verify_token(&mut tx, user_id, &account.email, now).await?;
    tx.commit().await?;

    // Send only after commit: never mail a link that failed to persist.
    super::mail::dispatch(
        ctx,
        super::mail::verification(&ctx.public_url, &account.email, &verify_token),
    );
    audit(&ctx.db, Some(user_id), Some(user_id), "user.created", None).await;

    Ok(CreatedAccount { user_id, public_id })
}

pub(crate) async fn verify_email(pool: &SqlitePool, token: VerifyToken) -> Result<()> {
    let now = now_ms();
    let mut tx = pool.begin().await?;

    // Consuming the token is the first statement, so two concurrent clicks
    // can't both succeed. If a later step fails, the rollback un-consumes it.
    let token_taken = db::auth::take_valid_token(
        &mut tx,
        TokenPurpose::VerifyEmail,
        &sha256_hex(token.expose_secret().trim()),
        now,
    )
    .await?;

    let Some(user_id) = token_taken.and_then(|t| t.user_id) else {
        return Err(AppError::BadRequest("invalid or expired token".into()));
    };

    if !db::auth::mark_email_verified(&mut tx, user_id, now).await? {
        return Err(AppError::BadRequest("invalid or expired token".into()));
    }

    tx.commit().await?;
    audit(pool, Some(user_id), Some(user_id), "email.verified", None).await;
    Ok(())
}

async fn issue_verify_token(
    conn: &mut SqliteConnection,
    user_id: i64,
    email: &str,
    now: i64,
) -> Result<VerifyToken> {
    let token = VerifyToken::new()?;
    db::auth::insert_token(
        conn,
        NewAuthToken {
            user_id: Some(user_id),
            purpose: TokenPurpose::VerifyEmail,
            token_hash: &sha256_hex(token.expose_secret()),
            target: Some(email),
            expires_at: add_ms(now, DAY_MS),
        },
    )
    .await?;
    Ok(token)
}
