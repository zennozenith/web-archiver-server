use std::{collections::HashSet, time::Duration};

use serde::Serialize;

use crate::error::{AppError, ForbiddenReason};

mod account;
mod api_key;
mod client;
pub(crate) mod policy;
mod profile;
mod token;
mod view;

pub(crate) use account::{AccountStatus, LoginCredentials, NewAccount};
pub(crate) use api_key::{CreatedApiKey, NewApiKey};
pub(crate) use client::ClientInfo;
pub(crate) use profile::ProfileUpdate;
pub(crate) use token::{
    ApiKeyToken, RandomToken, ResetToken, Rng, SessionToken, TokenPurpose, VerifyToken,
    generate_secret,
};
pub(crate) use view::{
    ApiKeyView, AuditEntry, ProfileView, RoleView, SessionView, UserFilter, UserView,
};

pub(crate) const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// A permission string known at compile time. `Permission::TAG_WRITE` instead of `"tag:write"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Permission(&'static str);

impl Permission {
    pub(crate) const ARCHIVE_READ: Self = Self("archive:read");
    pub(crate) const ARCHIVE_WRITE: Self = Self("archive:write");
    pub(crate) const ARCHIVE_ADMIN: Self = Self("archive:admin");
    pub(crate) const PROFILE_READ: Self = Self("profile:read");
    pub(crate) const PROFILE_WRITE: Self = Self("profile:write");
    pub(crate) const USERS_READ: Self = Self("users:read");
    pub(crate) const USERS_WRITE: Self = Self("users:write");
    pub(crate) const USERS_DELETE: Self = Self("users:delete");
    pub(crate) const ROLES_READ: Self = Self("roles:read");
    pub(crate) const ROLES_WRITE: Self = Self("roles:write");
    pub(crate) const AUDIT_READ: Self = Self("audit:read");
    /// Grants every permission; only super-admins hold it.
    pub(crate) const WILDCARD: Self = Self("*");

    pub(crate) const fn as_str(self) -> &'static str {
        self.0
    }
}

/// What the client presented. Parsing it from headers is the HTTP layer's job.
pub(crate) enum Credential {
    Session(SessionToken),
    ApiKey(ApiKeyToken),
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
        profile: AuthIdentityRow,
        roles: Vec<String>,
        permissions: HashSet<String>,
        origin: AuthMethod,
    ) -> Self {
        let (session_public_id, via_api_key) = match origin {
            AuthMethod::Session { public_id } => (Some(public_id), false),
            AuthMethod::ApiKey => (None, true),
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
    /// Wildcard holder, e.g. `super_admin`.
    pub(crate) fn is_super_admin(&self) -> bool {
        self.permissions.contains(Permission::WILDCARD.as_str())
    }

    /// Same as `can`, for permission names only known at runtime (API key scopes).
    pub(crate) fn has_scope(&self, name: &str) -> bool {
        self.is_super_admin() || self.permissions.contains(name)
    }

    pub(crate) fn can(&self, permission: Permission) -> bool {
        self.has_scope(permission.as_str())
    }

    pub(crate) fn require(&self, permission: Permission) -> Result<(), AppError> {
        if self.can(permission) {
            Ok(())
        } else {
            Err(AppError::Forbidden(ForbiddenReason::MissingPermission))
        }
    }
}

/// The columns both credential paths share.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct AuthIdentityRow {
    pub id: i64,
    pub public_id: String,
    pub email: Option<String>,
    pub username: Option<String>,
}

pub(crate) enum AuthMethod {
    Session { public_id: String },
    ApiKey,
}
