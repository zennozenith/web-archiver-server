use sqlx::{Result, SqliteExecutor};

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
