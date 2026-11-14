use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use sqlx::SqlitePool;

use crate::{
    db::{
        self,
        audit::{AuditAction, audit},
        auth::{NewAuthToken, ValidToken},
    },
    identity::{ResetToken, TokenPurpose, password_policy},
    infra::{hash_password, sha256_hex},
    prelude::*,
    util::time::{HOUR, add_ms, now_ms},
};

pub(crate) async fn reset_password(
    pool: &SqlitePool,
    token: ResetToken,
    new_password: SecretString,
) -> Result<()> {
    password_policy::ensure_password(&new_password)?;
    let now = now_ms();
    let mut tx = pool.begin().await?;

    let token = db::auth::find_valid_token(
        &mut tx,
        TokenPurpose::ResetPassword,
        &sha256_hex(token.expose_secret().trim()),
        now,
    )
    .await?;

    let Some(ValidToken {
        id: token_id,
        user_id: Some(user_id),
    }) = token
    else {
        return Err(AppError::BadRequest("invalid or expired token".into()));
    };

    let recent = db::auth::recent_password_hashes(&mut tx, user_id).await?;
    if password_policy::password_was_recently_used(recent, &new_password).await {
        return Err(AppError::BadRequest(
            "choose a password you have not used recently".into(),
        ));
    }

    let hash = hash_password(new_password).await?;
    db::auth::insert_password_history_from_current(&mut tx, user_id).await?;
    db::auth::upsert_password(&mut tx, user_id, &hash, now).await?;
    db::auth::consume_token(&mut tx, token_id, now).await?;
    db::auth::revoke_all_sessions(&mut tx, user_id, "password_reset", now).await?; // signs out everywhere
    tx.commit().await?;

    audit(
        pool,
        Some(user_id),
        Some(user_id),
        AuditAction::PasswordChanged,
        Some(json!({"via":"reset"})),
    )
    .await;
    Ok(())
}

/// Starts a password reset.
///
/// Always returns `Ok(())` for unknown, inactive, or blank e-mails, so the
/// caller can show the same "if that account exists, we sent a link" message
/// and the response can't be used to enumerate accounts.
pub(crate) async fn forgot_password(state: &AppState, email: String) -> Result<()> {
    let email = email.trim();
    if email.is_empty() {
        return Err(AppError::BadRequest("Email empty".into()));
    }

    let Some(user_id) = db::auth::find_active_user_id_by_email(&state.db, email).await? else {
        return Ok(());
    };

    let now = now_ms();
    let token = ResetToken::new()?;

    let mut tx = state.db.begin().await?;
    // Only the most recent link should work.
    db::auth::invalidate_open_tokens(&mut tx, user_id, TokenPurpose::ResetPassword, now).await?;
    db::auth::insert_token(
        &mut tx,
        NewAuthToken {
            user_id: Some(user_id),
            purpose: TokenPurpose::ResetPassword,
            token_hash: &sha256_hex(token.expose_secret()),
            target: Some(email),
            expires_at: add_ms(now, HOUR),
        },
    )
    .await?;
    tx.commit().await?;

    // Send only after commit: never mail a link that failed to persist.
    super::mail::dispatch(
        state,
        super::mail::password_reset(&state.server.public_url, email, &token),
    );
    audit(
        &state.db,
        Some(user_id),
        Some(user_id),
        AuditAction::PasswordResetRequested,
        None,
    )
    .await;

    Ok(())
}
