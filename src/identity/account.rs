use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::error::ForbiddenReason;

#[derive(Deserialize)]
pub(crate) struct NewAccount {
    pub email: String,
    pub username: Option<String>,
    pub password: SecretString,
    pub display_name: Option<String>,
}

/// Validate an e-mail address the way the `users` table expects.
///
/// Lengths count characters, not bytes, so a non-ASCII local part is not
/// penalised. Returns a human-readable reason so the CLI can re-prompt.
pub(crate) fn validate_email(email: &str) -> Result<(), String> {
    let len = email.chars().count();
    if !(3..=254).contains(&len) {
        return Err("email must be 3-254 characters".to_owned());
    }
    if email.chars().any(char::is_whitespace) {
        return Err("email must not contain whitespace".to_owned());
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err("email must contain '@'".to_owned());
    };
    let chars: Vec<char> = domain.chars().collect();
    let dot_ok = chars
        .iter()
        .enumerate()
        .any(|(i, c)| *c == '.' && i >= 1 && i < chars.len().saturating_sub(1));
    if local.is_empty() || !dot_ok {
        return Err("email must look like name@example.com".to_owned());
    }
    Ok(())
}

/// Validate a username against the `users` table's CHECK constraint.
pub(crate) fn validate_username(username: &str) -> Result<(), String> {
    let len = username.chars().count();
    if !(3..=32).contains(&len) {
        return Err("username must be 3-32 characters".to_owned());
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err("username may only contain letters, digits, '_', '.', '-'".to_owned());
    }
    Ok(())
}

#[derive(
    Debug,
    strum::Display,
    strum::EnumString,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    sqlx::Type,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub(crate) enum AccountStatus {
    Pending,
    Active,
    Suspended,
    Deactivated,
}

impl From<AccountStatus> for ForbiddenReason {
    fn from(value: AccountStatus) -> Self {
        match value {
            AccountStatus::Pending => Self::EmailUnverified,
            AccountStatus::Suspended => Self::Suspended,
            AccountStatus::Deactivated => Self::Deactivated,
            // Callers only convert non-active statuses into a 403. Keep the
            // match exhaustive instead of adding an "unknown" fallback arm.
            AccountStatus::Active => Self::MissingPermission,
        }
    }
}

/// What `service::auth::login` needs. Build it with `new`.
pub(crate) struct LoginCredentials {
    identifier: String, // e-mail or username, trimmed
    password: SecretString,
}

impl LoginCredentials {
    pub(crate) fn new(identifier: &str, password: SecretString) -> Self {
        Self {
            identifier: identifier.trim().to_owned(),
            password,
        }
    }

    pub(crate) fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Consumes self because `verify_password` takes the password by value.
    pub(crate) fn into_password(self) -> SecretString {
        self.password
    }
}
