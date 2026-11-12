use secrecy::{ExposeSecret as _, SecretString};

use crate::{error::ForbiddenReason, infra::verify_password, prelude::*};

pub(crate) const DEFAULT_ROLE: &str = "user";
pub(crate) const MAX_FAILED_ATTEMPTS: i64 = 5;
pub(crate) const LOCKOUT_MS: i64 = 15 * 60 * 1000;
pub(crate) const PASSWORD_MIN: usize = 8;
pub(crate) const PASSWORD_MAX: usize = 128;
pub(crate) const TERMS_VERSION: &str = "2026-01";

pub(crate) fn check_password_len(password: &SecretString) -> Result<()> {
    // chars, not bytes, so a non-ASCII passphrase isn't penalised
    let n = password.expose_secret().chars().count();
    if (PASSWORD_MIN..=PASSWORD_MAX).contains(&n) {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "password must be between {PASSWORD_MIN} and {PASSWORD_MAX} characters"
        )))
    }
}

pub(crate) fn inactive_error(status: &str) -> AppError {
    let reason = match status {
        "pending" => ForbiddenReason::Pending,
        "suspended" => ForbiddenReason::Suspended,
        "deactivated" => ForbiddenReason::Deactivated,
        _ => ForbiddenReason::Unknown,
    };
    AppError::Forbidden(reason)
}

pub(crate) async fn was_recently_used(
    recent: Vec<String>,
    new_password: &secrecy::SecretBox<str>,
) -> bool {
    for hash in recent {
        if verify_password(new_password.clone(), hash).await {
            return true;
        }
    }
    false
}
