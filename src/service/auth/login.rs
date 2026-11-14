use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use tokio::sync::OnceCell;

use crate::{
    auth::{AccountStatus, ClientInfo, LoginCredentials, SessionToken, policy},
    db::{self, audit::audit},
    error::ForbiddenReason,
    infra::{hash_password, random_id, sha256_hex, verify_password},
    prelude::*,
    service::auth::attempt::AttemptLog,
    util::time::now_ms,
};

static DECOY_HASH: OnceCell<String> = OnceCell::const_new();

// Hash of a random password nobody knows, made with the live Argon2 params.
pub(super) async fn decoy_hash() -> &'static str {
    DECOY_HASH
        .get_or_init(|| async {
            let pw = SecretString::from(random_id().expect("rng available"));
            hash_password(pw).await.expect("hash decoy password")
        })
        .await
}

/// Spend the same time as a real verification, and discard the result.
async fn burn_verify(password: SecretString) {
    let _ = verify_password(password, decoy_hash().await.to_owned()).await;
}

pub(crate) async fn login(
    ctx: &AppState,
    client: &ClientInfo,
    creds: LoginCredentials,
) -> Result<SessionToken> {
    let now = now_ms();
    let identifier = creds.identifier().to_owned();
    let password = creds.into_password();

    let attempt = AttemptLog::new(&ctx.db, &identifier, client);

    let Some(user) = db::auth::find_login_user(&ctx.db, &identifier).await? else {
        burn_verify(password.clone()).await;
        return Err(attempt
            .failed(None, "unknown_user", AppError::InvalidCredentials)
            .await);
    };

    if user.locked_until.is_some_and(|t| t > now) {
        // Run the same expensive verification as a normal failed login, then
        // return the generic error so the response does not reveal whether the
        // account exists or is locked.
        burn_verify(password).await;
        return Err(attempt
            .failed(Some(user.id), "locked", AppError::InvalidCredentials)
            .await);
    }
    if !verify_password(password, user.password_hash).await {
        db::auth::record_bad_password(
            &ctx.db,
            user.id,
            now,
            policy::MAX_FAILED_ATTEMPTS,
            policy::LOCKOUT_MS,
        )
        .await?;
        return Err(attempt
            .failed(Some(user.id), "bad_password", AppError::InvalidCredentials)
            .await);
    }
    if user.status != AccountStatus::Active {
        return Err(attempt
            .failed(
                Some(user.id),
                "inactive",
                ForbiddenReason::from(user.status).into(),
            )
            .await);
    }
    let token = SessionToken::new()?;
    let token_hash = sha256_hex(token.expose_secret());
    let token_id = random_id()?;

    db::auth::create_session(&ctx.db, user.id, client, &token_id, &token_hash, now).await?;

    attempt.succeeded(user.id).await;
    audit(
        &ctx.db,
        Some(user.id),
        Some(user.id),
        "auth.login",
        Some(json!({ "ip": client.ip, "user_agent": &client.user_agent })),
    )
    .await;

    Ok(token)
}

pub(crate) async fn logout(ctx: &AppState, session_token: SessionToken) -> Result<()> {
    let now = now_ms();

    db::auth::delete_session(&ctx.db, session_token, now)
        .await
        .map_err(AppError::from)
}
