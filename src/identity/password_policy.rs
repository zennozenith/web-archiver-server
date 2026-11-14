use secrecy::{ExposeSecret as _, SecretString};

use crate::{infra::verify_password, prelude::*};

/// The one password policy used by registration, reset, and the admin CLI.
pub(crate) const PASSWORD_MIN: usize = 12;
pub(crate) const PASSWORD_MAX: usize = 128;

/// Passwords that lead every breach corpus. This is a small, curated subset of
/// the well-known lists; a full deployment should replace it with a breached
/// password service. Matching is exact after whitespace/lowercase folding so a
/// legitimate passphrase that merely contains an ordinary word still passes.
const COMMON_PASSWORDS: &[&str] = &[
    "123456789012",
    "1234567890123",
    "12345678901234",
    "123456789012345",
    "1234567890123456",
    "12345678901234567",
    "123456789012345678",
    "abc123456789",
    "abcdefghijkl",
    "admin123456",
    "admin1234567",
    "admin12345678",
    "changeme1234",
    "iloveyou1234",
    "letmein12345",
    "letmein123456",
    "monkey123456",
    "password1234",
    "password12345",
    "password123456",
    "password1234567",
    "password12345678",
    "qwerty123456",
    "qwertyuiop12",
    "qwertyuiop123",
    "qwertyuiopasd",
    "sunshine1234",
    "trustno1trustno1",
    "welcome12345",
    "welcome123456",
    "welcome1234567",
];

fn is_common_password(password: &str) -> bool {
    let normalized: String = password
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    COMMON_PASSWORDS.contains(&normalized.as_str())
}

/// Validate an already-open password. Returns a human-readable reason so the
/// admin CLI can re-prompt without depending on HTTP error types.
///
/// Follows current NIST guidance (SP 800-63B): length plus a common-password
/// check, without composition rules that push users toward predictable
/// substitutions.
pub(crate) fn validate_password(password: &str) -> Result<(), String> {
    // chars, not bytes, so a non-ASCII passphrase isn't penalised
    let n = password.chars().count();
    if !(PASSWORD_MIN..=PASSWORD_MAX).contains(&n) {
        return Err(format!(
            "password must be between {PASSWORD_MIN} and {PASSWORD_MAX} characters"
        ));
    }
    if is_common_password(password) {
        return Err("password is too common; choose something less predictable".to_owned());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_long_uncommon_passphrase() {
        assert!(validate_password("correct horse battery staple").is_ok());
    }

    #[test]
    fn rejects_short_and_overlong_passwords() {
        assert!(validate_password("short").is_err());
        assert!(validate_password(&"x".repeat(PASSWORD_MAX + 1)).is_err());
    }

    #[test]
    fn rejects_common_passwords_case_and_space_folded() {
        assert!(validate_password("Password123456").is_err());
        assert!(validate_password("password 123456").is_err());
    }
}
