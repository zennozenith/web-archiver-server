use std::collections::HashSet;

use secrecy::SecretString;
use serde::Serialize;

use crate::{
    error::{AppError, ForbiddenReason},
    util::time::DAY_MS,
};

mod account;
mod client;
pub(crate) mod policy;
mod token;

pub(crate) use account::{AccountStatus, CreatedAccount, LoginCredentials, NewAccount};
pub(crate) use client::ClientInfo;
pub(crate) use token::{
    ApiKeyToken, RandomToken, ResetToken, Rng, SessionToken, TokenPurpose, VerifyToken,
    generate_secret,
};

pub(crate) const SESSION_TTL_MS: i64 = 7 * DAY_MS;

/// A permission string known at compile time. `Permission::TAG_WRITE` instead of `"tag:write"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Permission(&'static str);

impl Permission {
    pub(crate) const TAG_READ: Self = Self("tag:read");
    pub(crate) const TAG_WRITE: Self = Self("tag:write");
    // add the rest here, one line each

    pub(crate) const fn as_str(self) -> &'static str {
        self.0
    }
}

/// What the client presented. Parsing it from headers is the HTTP layer's job.
pub(crate) enum Credential {
    Session(SessionToken),
    // TODO: replace with ApiKeyToken
    ApiKey(SecretString),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthUser {
    pub id: i64,
    pub public_id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    pub roles: Vec<String>,
    #[serde(skip)]
    permissions: HashSet<String>,
    /// Set for cookie sessions (used to flag "current session").
    pub session_public_id: Option<String>,
    pub via_api_key: bool,
}

impl AuthUser {
    pub(crate) fn new(
        profile: UserProfile,
        roles: Vec<String>,
        permissions: HashSet<String>,
        origin: Origin,
    ) -> Self {
        let (session_public_id, via_api_key) = match origin {
            Origin::Session { public_id } => (Some(public_id), false),
            Origin::ApiKey => (None, true),
        };
        Self {
            id: profile.id,
            public_id: profile.public_id,
            email: profile.email,
            username: profile.username,
            roles,
            permissions,
            session_public_id,
            via_api_key,
        }
    }

    pub(crate) fn can(&self, permission: Permission) -> bool {
        self.permissions.contains("*") || self.permissions.contains(permission.as_str())
    }

    pub(crate) fn require(&self, permission: Permission) -> Result<(), AppError> {
        if self.can(permission) {
            Ok(())
        } else {
            Err(AppError::Forbidden(ForbiddenReason::Permission))
        }
    }

    pub(crate) fn permissions(&self) -> impl Iterator<Item = &str> {
        self.permissions.iter().map(String::as_str)
    }
}

/// The columns both credential paths share.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct UserProfile {
    pub id: i64,
    pub public_id: String,
    pub email: Option<String>,
    pub username: Option<String>,
}

pub(crate) enum Origin {
    Session { public_id: String },
    ApiKey,
}
