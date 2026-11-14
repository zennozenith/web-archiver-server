use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use crate::{error::ForbiddenReason, identity::AuthUser, infra::verify_password, prelude::*};

pub(crate) const SUPER_ADMIN_ROLE: &str = "super_admin";
pub(crate) const DEFAULT_ROLE: &str = "user";
pub(crate) const MAX_FAILED_ATTEMPTS: i64 = 5;
pub(crate) const LOCKOUT: Duration = Duration::from_secs(15 * 60);
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

/// Only a wildcard holder may hand out or take away the top role.
pub(crate) fn check_role_assignable(actor: &AuthUser, role: &str) -> Result<()> {
    if role == SUPER_ADMIN_ROLE && !actor.is_super_admin() {
        return Err(AppError::Forbidden(ForbiddenReason::MissingPermission));
    }
    Ok(())
}

pub(crate) fn check_not_self(actor_id: i64, target_id: i64, message: &'static str) -> Result<()> {
    if actor_id == target_id {
        Err(AppError::BadRequest(message.into()))
    } else {
        Ok(())
    }
}
