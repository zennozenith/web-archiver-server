use serde_json::Value;
use sqlx::{SqliteExecutor, SqlitePool};

use crate::{identity::AuditEntry, util::like_contains_pattern};

/// Every audit action the app writes. Typed so a typo cannot silently create
/// a new, unfilterable action name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditAction {
    AuthLogin,
    UserCreated,
    EmailVerified,
    PasswordChanged,
    PasswordResetRequested,
    UserStatusChanged,
    UserDeleted,
    RoleGranted,
    RoleRevoked,
    ProfileUpdated,
    SessionRevoked,
    ApiKeyCreated,
    ApiKeyRevoked,
}

impl AuditAction {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AuthLogin => "auth.login",
            Self::UserCreated => "user.created",
            Self::EmailVerified => "email.verified",
            Self::PasswordChanged => "password.changed",
            Self::PasswordResetRequested => "password.reset_requested",
            Self::UserStatusChanged => "user.status_changed",
            Self::UserDeleted => "user.deleted",
            Self::RoleGranted => "role.granted",
            Self::RoleRevoked => "role.revoked",
            Self::ProfileUpdated => "profile.updated",
            Self::SessionRevoked => "session.revoked",
            Self::ApiKeyCreated => "api_key.created",
            Self::ApiKeyRevoked => "api_key.revoked",
        }
    }
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Append a row to the append-only `audit_log`. Failures are logged, never bubbled up.
pub(crate) async fn audit<'e>(
    executor: impl SqliteExecutor<'e>,
    actor: Option<i64>,
    target: Option<i64>,
    action: AuditAction,
    details: Option<Value>,
) {
    let res = sqlx::query(
        "INSERT INTO audit_log (actor_user_id, target_user_id, action, details) VALUES (?, ?, ?, ?)",
    )
    .bind(actor)
    .bind(target)
    .bind(action.as_str())
    .bind(details.map_or_else(|| String::from("{}"), |v| v.to_string()))
    .execute(executor)
    .await;
    if let Err(e) = res {
        tracing::warn!("failed to write audit log ({action}): {e}");
    }
}

pub(crate) async fn list(pool: &SqlitePool, limit: i64) -> Result<Vec<AuditEntry>, sqlx::Error> {
    let rows: Vec<AuditEntry> = sqlx::query_as(
        "SELECT a.id, a.action, actor.email AS actor_email, target.email AS target_email,
                a.details, a.created_at
           FROM audit_log a
           LEFT JOIN users actor  ON actor.id  = a.actor_user_id
           LEFT JOIN users target ON target.id = a.target_user_id
          ORDER BY a.id DESC
          LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

macro_rules! audit_select {
    () => {
        "SELECT a.id, a.action, actor.email AS actor_email,
        target.email AS target_email, a.details, a.created_at
        FROM audit_log a
        LEFT JOIN users actor  ON actor.id  = a.actor_user_id
        LEFT JOIN users target ON target.id = a.target_user_id"
    };
}

pub(crate) async fn search(
    pool: &SqlitePool,
    q: &str,
    before: Option<i64>,
    limit: i64,
) -> sqlx::Result<Vec<AuditEntry>> {
    let pattern = (!q.is_empty()).then(|| like_contains_pattern(q));
    sqlx::query_as(concat!(
        audit_select!(),
        " WHERE (?1 IS NULL OR a.action LIKE ?1 ESCAPE '\\'
                         OR actor.email LIKE ?1 ESCAPE '\\'
                         OR target.email LIKE ?1 ESCAPE '\\')
            AND (?2 IS NULL OR a.id < ?2)
          ORDER BY a.id DESC LIMIT ?3"
    ))
    .bind(pattern)
    .bind(before)
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub(crate) async fn for_user(
    pool: &SqlitePool,
    user_id: i64,
    limit: i64,
) -> sqlx::Result<Vec<AuditEntry>> {
    sqlx::query_as(concat!(
        audit_select!(),
        " WHERE a.target_user_id = ?1 OR a.actor_user_id = ?1
          ORDER BY a.id DESC LIMIT ?2"
    ))
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}
