//! Read models for the account area, plus validated list filters.
//! Plain data: no axum, no wire formatting. `http/dto` turns these into JSON.

use sqlx::FromRow;

use crate::{auth::AccountStatus, prelude::*};

fn split_csv(csv: Option<&str>) -> Vec<String> {
    csv.map_or_default(|s| {
        s.split(',')
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

#[derive(Debug, FromRow)]
pub(crate) struct UserView {
    pub public_id: String,
    pub email: Option<String>,
    pub email_verified_at: Option<i64>,
    pub username: Option<String>,
    pub phone_e164: Option<String>,
    pub status: AccountStatus,
    pub last_login_at: Option<i64>,
    pub created_at: i64,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    roles: Option<String>, // group_concat from the query
}

impl UserView {
    pub(crate) fn roles(&self) -> Vec<String> {
        split_csv(self.roles.as_deref())
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ProfileView {
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub birth_date: Option<String>,
    pub locale: String,
    pub timezone: String,
    pub updated_at: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct SessionView {
    pub public_id: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub expires_at: i64,
    /// Not a column: the service sets it by comparing with the caller's session.
    #[sqlx(skip)]
    pub current: bool,
}

#[derive(Debug, FromRow)]
pub(crate) struct ApiKeyView {
    pub public_id: String,
    pub name: String,
    pub key_prefix: String,
    scopes: String, // JSON array
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
}

impl ApiKeyView {
    pub(crate) fn scopes(&self) -> Vec<String> {
        serde_json::from_str(&self.scopes).unwrap_or_default()
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct AuditEntry {
    pub id: i64,
    pub action: String,
    pub actor_email: Option<String>,
    pub target_email: Option<String>,
    pub details: String, // JSON text
    pub created_at: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct RoleView {
    pub name: String,
    pub description: Option<String>,
    pub is_system: bool,
    permissions: Option<String>,
}

impl RoleView {
    pub(crate) fn permissions(&self) -> Vec<String> {
        split_csv(self.permissions.as_deref())
    }
}

/// Admin user list filter. Built with `new`, so limits are always sane.
#[derive(Debug)]
pub(crate) struct UserFilter {
    /// Ready-made `LIKE` pattern (escaped), `None` = no text filter.
    pub pattern: Option<String>,
    pub status: Option<AccountStatus>,
    pub limit: i64,
    pub offset: i64,
}

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

impl UserFilter {
    pub(crate) fn new(
        search: Option<&str>,
        status: Option<&str>,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Self> {
        let pattern = search
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(like_contains);

        let status = match status.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => Some(
                s.parse::<AccountStatus>()
                    .map_err(|_| AppError::BadRequest("unknown status".into()))?,
            ),
            None => None,
        };

        Ok(Self {
            pattern,
            status,
            limit: limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
            offset: offset.unwrap_or(0).max(0),
        })
    }
}

/// `%text%` with `%`, `_` and `\` escaped (the query uses `ESCAPE '\'`).
fn like_contains(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(2));
    out.push('%');
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}
