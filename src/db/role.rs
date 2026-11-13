use sqlx::{Result, SqliteExecutor, SqlitePool, prelude::FromRow};

use crate::auth::RoleView;

pub(crate) async fn list<'e>(ex: impl SqliteExecutor<'e>) -> Result<Vec<RoleView>> {
    sqlx::query_as(
        "SELECT r.name, r.description, r.is_system,
                (SELECT group_concat(p.name)
                   FROM role_permissions rp JOIN permissions p ON p.id = rp.permission_id
                  WHERE rp.role_id = r.id) AS permissions
           FROM roles r
          ORDER BY r.id",
    )
    .fetch_all(ex)
    .await
}

pub(crate) async fn find_id<'e>(ex: impl SqliteExecutor<'e>, name: &str) -> Result<Option<i64>> {
    sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
        .bind(name)
        .fetch_optional(ex)
        .await
}

/// `false` if the user already had the role.
pub(crate) async fn grant<'e>(
    ex: impl SqliteExecutor<'e>,
    user_id: i64,
    role_id: i64,
    granted_by: i64,
) -> Result<bool> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO user_roles (user_id, role_id, granted_by) VALUES (?, ?, ?)",
    )
    .bind(user_id)
    .bind(role_id)
    .bind(granted_by)
    .execute(ex)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// `false` if the user didn't have that role (or it doesn't exist).
pub(crate) async fn revoke<'e>(
    ex: impl SqliteExecutor<'e>,
    user_id: i64,
    role: &str,
) -> Result<bool> {
    let res = sqlx::query(
        "DELETE FROM user_roles
          WHERE user_id = ? AND role_id = (SELECT id FROM roles WHERE name = ?)",
    )
    .bind(user_id)
    .bind(role)
    .execute(ex)
    .await?;
    Ok(res.rows_affected() > 0)
}

#[derive(Debug, FromRow)]
pub(crate) struct RoleSummary {
    pub name: String,
    pub description: Option<String>,
    pub is_system: bool,
    pub members: i64,
    /// Comma-separated; use [`RoleSummary::permissions`].
    permissions: Option<String>,
}

impl RoleSummary {
    pub(crate) fn permissions(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .permissions
            .as_deref()
            .map_or_else(Vec::new, |s| s.split(',').map(str::to_owned).collect());
        out.sort();
        out
    }
}

pub(crate) async fn summaries(pool: &SqlitePool, now: i64) -> sqlx::Result<Vec<RoleSummary>> {
    sqlx::query_as(
        "SELECT r.name, r.description, r.is_system,
                (SELECT COUNT(*) FROM user_roles ur JOIN users u ON u.id = ur.user_id
                  WHERE ur.role_id = r.id AND u.deleted_at IS NULL
                    AND (ur.expires_at IS NULL OR ur.expires_at > ?)) AS members,
                (SELECT group_concat(p.name) FROM role_permissions rp
                   JOIN permissions p ON p.id = rp.permission_id
                  WHERE rp.role_id = r.id) AS permissions
           FROM roles r ORDER BY r.id",
    )
    .bind(now)
    .fetch_all(pool)
    .await
}
