use std::str::FromStr;

use serde::Deserialize;
use serde_json::json;
use sqlx::{FromRow, SqlitePool};

use crate::{
    auth::AccountStatus,
    db::{audit::audit, user::UserRow},
    error::AppError,
    prelude::*,
    user_select,
    util::time::now_ms,
};

async fn user_id_by_public(pool: &SqlitePool, public_id: &str) -> Result<i64> {
    sqlx::query_scalar("SELECT id FROM users WHERE public_id = ? AND deleted_at IS NULL")
        .bind(public_id)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::NotFound("user"))
}

// ----------------------------------------------------------------- users

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    q: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

pub(crate) async fn list_users(pool: &SqlitePool, q: ListQuery) -> Result<Vec<UserRow>> {
    let like = format!("%{}%", q.q.as_deref().unwrap_or("").trim());
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let status = q.status.filter(|s| !s.is_empty());

    let rows: Vec<UserRow> = sqlx::query_as(concat!(
        user_select!(),
        " WHERE (?1 = '%%' OR email LIKE ?1 OR username LIKE ?1 OR display_name LIKE ?1)
            AND (?2 IS NULL OR status = ?2)
          ORDER BY created_at DESC
          LIMIT ?3 OFFSET ?4"
    ))
    .bind(&like)
    .bind(status)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[derive(Deserialize)]
pub(crate) struct StatusReq {
    pub(crate) status: String,
    pub(crate) reason: Option<String>,
}

pub(crate) async fn set_status(
    pool: &SqlitePool,
    admin_id: i64,
    public_id: String,
    req: StatusReq,
) -> Result<()> {
    if AccountStatus::from_str(&req.status).is_err() {
        return Err(AppError::BadRequest("unknown status".into()));
    }
    let target = user_id_by_public(pool, &public_id).await?;
    if target == admin_id {
        return Err(AppError::BadRequest(
            "you cannot change your own status".into(),
        ));
    }

    // Moving to suspended/deactivated fires trg_users_revoke_sessions,
    // which revokes all of the user's live sessions inside the database.
    let reason = req
        .reason
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty());
    sqlx::query("UPDATE users SET status = ?, status_reason = ? WHERE id = ?")
        .bind(&req.status)
        .bind(&reason)
        .bind(target)
        .execute(pool)
        .await?;

    audit(
        pool,
        Some(admin_id),
        Some(target),
        "user.status_changed",
        Some(json!({ "status": req.status, "reason": reason })),
    )
    .await;
    Ok(())
}

pub(crate) async fn delete_user(pool: &SqlitePool, admin_id: i64, public_id: String) -> Result<()> {
    let target = user_id_by_public(pool, &public_id).await?;
    if target == admin_id {
        return Err(AppError::BadRequest("you cannot delete yourself".into()));
    }

    // Soft delete: the partial unique indexes free the e-mail/username for reuse.
    sqlx::query("UPDATE users SET deleted_at = ?, status = 'deactivated' WHERE id = ?")
        .bind(now_ms())
        .bind(target)
        .execute(pool)
        .await?;

    audit(pool, Some(admin_id), Some(target), "user.deleted", None).await;
    Ok(())
}

// ----------------------------------------------------------------- roles

pub(crate) async fn grant_role(
    pool: &SqlitePool,
    admin_id: i64,
    public_id: String,
    role: String,
) -> Result<()> {
    let target = user_id_by_public(pool, &public_id).await?;
    let role_id: i64 = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
        .bind(&role)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::NotFound("role"))?;

    sqlx::query("INSERT OR IGNORE INTO user_roles (user_id, role_id, granted_by) VALUES (?, ?, ?)")
        .bind(target)
        .bind(role_id)
        .bind(admin_id)
        .execute(pool)
        .await?;

    audit(
        pool,
        Some(admin_id),
        Some(target),
        "role.granted",
        Some(json!({ "role": role })),
    )
    .await;
    Ok(())
}

pub(crate) async fn revoke_role(
    pool: &SqlitePool,
    admin_id: i64,
    public_id: String,
    role: String,
) -> Result<()> {
    let target = user_id_by_public(pool, &public_id).await?;

    let res = sqlx::query(
        "DELETE FROM user_roles
          WHERE user_id = ? AND role_id = (SELECT id FROM roles WHERE name = ?)",
    )
    .bind(target)
    .bind(&role)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("user_or_role"));
    }

    audit(
        pool,
        Some(admin_id),
        Some(target),
        "role.revoked",
        Some(json!({ "role": role })),
    )
    .await;
    Ok(())
}

#[derive(Debug, FromRow)]
pub(crate) struct RoleRow {
    pub name: String,
    pub description: Option<String>,
    pub is_system: i64,
    pub permissions: Option<String>,
}

pub(crate) async fn list_roles(pool: &SqlitePool) -> Result<Vec<RoleRow>> {
    let rows: Vec<RoleRow> = sqlx::query_as(
        "SELECT r.name, r.description, r.is_system,
                (SELECT group_concat(p.name)
                   FROM role_permissions rp JOIN permissions p ON p.id = rp.permission_id
                  WHERE rp.role_id = r.id) AS permissions
           FROM roles r
          ORDER BY r.id",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ----------------------------------------------------------------- audit

#[derive(Deserialize)]
pub(crate) struct AuditQuery {
    limit: Option<i64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct AuditRow {
    pub id: i64,
    pub action: String,
    pub actor_email: Option<String>,
    pub target_email: Option<String>,
    pub details: String,
    pub created_at: i64,
}

pub(crate) async fn list_audit(pool: &SqlitePool, q: AuditQuery) -> Result<Vec<AuditRow>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);

    let rows: Vec<AuditRow> = sqlx::query_as(
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
