use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    auth::{AccountStatus, AuditEntry, RoleView, UserFilter},
    error::AppError,
    util::time::ms_to_dt,
};

#[derive(Deserialize)]
pub(crate) struct StatusRequest {
    pub status: AccountStatus,
    pub reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ListUsersQuery {
    q: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

impl TryFrom<ListUsersQuery> for UserFilter {
    type Error = AppError;

    fn try_from(q: ListUsersQuery) -> Result<Self, Self::Error> {
        Self::new(q.q.as_deref(), q.status.as_deref(), q.limit, q.offset)
    }
}

#[derive(Deserialize)]
pub(crate) struct AuditQuery {
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub(crate) struct RoleDto {
    name: String,
    description: Option<String>,
    is_system: bool,
    permissions: Vec<String>,
}

impl From<RoleView> for RoleDto {
    fn from(r: RoleView) -> Self {
        let permissions = r.permissions();
        Self {
            name: r.name,
            description: r.description,
            is_system: r.is_system,
            permissions,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct AuditDto {
    id: i64,
    action: String,
    actor: Option<String>,
    target: Option<String>,
    details: Value,
    created_at: DateTime<Utc>,
}

impl From<AuditEntry> for AuditDto {
    fn from(e: AuditEntry) -> Self {
        Self {
            id: e.id,
            action: e.action,
            actor: e.actor_email,
            target: e.target_email,
            details: serde_json::from_str(&e.details).unwrap_or(Value::Null),
            created_at: ms_to_dt(e.created_at),
        }
    }
}
