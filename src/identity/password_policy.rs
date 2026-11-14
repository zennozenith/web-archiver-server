use secrecy::{ExposeSecret as _, SecretString};

use crate::{infra::verify_password, prelude::*};

/// The one password policy used by registration, reset, and the admin CLI.
pub(crate) const PASSWORD_MIN: usize = 12;
pub(crate) const PASSWORD_MAX: usize = 128;

/// Validate an already-open password. Returns a human-readable reason so the
/// admin CLI can re-prompt without depending on HTTP error types.
pub(crate) fn validate_password(password: &str) -> Result<(), String> {
    // chars, not bytes, so a non-ASCII passphrase isn't penalised
    let n = password.chars().count();
    if !(PASSWORD_MIN..=PASSWORD_MAX).contains(&n) {
        return Err(format!(
            "password must be between {PASSWORD_MIN} and {PASSWORD_MAX} characters"
        ));
    }

    let classes = [
        password.chars().any(char::is_lowercase),
        password.chars().any(char::is_uppercase),
        password.chars().any(|c| c.is_ascii_digit()),
        password.chars().any(|c| !c.is_alphanumeric()),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if classes < 3 {
        return Err("use at least 3 of: lowercase, uppercase, digits, symbols".to_owned());
    }
    Ok(())
}

pub(crate) fn ensure_password(password: &SecretString) -> Result<()> {
    validate_password(password.expose_secret()).map_err(AppError::BadRequest)
}

pub(crate) async fn password_was_recently_used(
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
