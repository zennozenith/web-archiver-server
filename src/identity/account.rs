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
