//! SQL for the `users` table / `v_users` view (admin and self-service reads).

use sqlx::{Result, SqliteExecutor, SqlitePool, prelude::FromRow};

use crate::auth::{AccountStatus, UserFilter, UserView};

/// Base query over `v_users` (live users + profile, never credentials), with
/// the active roles folded into one comma separated column.
#[macro_export]
macro_rules! user_select {
    () => {
        "SELECT id, public_id, email, email_verified_at, username, phone_e164, \
           status, last_login_at, created_at, display_name, first_name, last_name, avatar_url, locale, timezone, \
           (SELECT group_concat(r.role) FROM v_user_roles r WHERE r.user_id = v_users.id) AS roles \
       FROM v_users"
    };
}

pub(crate) async fn find<'e>(ex: impl SqliteExecutor<'e>, user_id: i64) -> Result<UserView> {
    sqlx::query_as(concat!(user_select!(), " WHERE id = ?"))
        .bind(user_id)
        .fetch_one(ex)
        .await
}

pub(crate) async fn find_id_by_public_id<'e>(
    ex: impl SqliteExecutor<'e>,
    public_id: &str,
) -> Result<Option<i64>> {
    sqlx::query_scalar("SELECT id FROM users WHERE public_id = ? AND deleted_at IS NULL")
        .bind(public_id)
        .fetch_optional(ex)
        .await
}

macro_rules! user_filter {
    () => {
        " WHERE (?1 IS NULL
                    OR email        LIKE ?1 ESCAPE '\\'
                    OR username     LIKE ?1 ESCAPE '\\'
                    OR display_name LIKE ?1 ESCAPE '\\')
               AND (?2 IS NULL OR status = ?2)
               AND (?3 IS NULL OR id IN (SELECT user_id FROM v_user_roles WHERE role = ?3))"
    };
}

pub(crate) async fn list<'e>(
    ex: impl SqliteExecutor<'e>,
    filter: &UserFilter,
) -> Result<Vec<UserView>> {
    sqlx::query_as(concat!(
        user_select!(),
        user_filter!(),
        " ORDER BY created_at DESC LIMIT ?4 OFFSET ?5"
    ))
    .bind(filter.like_pattern())
    .bind(filter.status)
    .bind(filter.role.as_deref())
    .bind(filter.limit)
    .bind(filter.offset)
    .fetch_all(ex)
    .await
}

/// Moving to suspended/deactivated fires `trg_users_revoke_sessions`, which
/// revokes the user's live sessions inside the database.
/// Returns `false` if the user doesn't exist or is deleted.
pub(crate) async fn set_status<'e>(
    ex: impl SqliteExecutor<'e>,
    user_id: i64,
    status: AccountStatus,
    reason: Option<&str>,
) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE users SET status = ?, status_reason = ? WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(status)
    .bind(reason)
    .bind(user_id)
    .execute(ex)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Soft delete: the partial unique indexes free the e-mail/username for reuse.
pub(crate) async fn soft_delete<'e>(
    ex: impl SqliteExecutor<'e>,
    user_id: i64,
    now: i64,
) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE users SET deleted_at = ?, status = 'deactivated'
          WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(ex)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub(crate) async fn count(pool: &SqlitePool, filter: &UserFilter) -> sqlx::Result<i64> {
    sqlx::query_scalar(concat!("SELECT COUNT(*) FROM v_users", user_filter!()))
        .bind(filter.like_pattern())
        .bind(filter.status)
        .bind(filter.role.as_deref())
        .fetch_one(pool)
        .await
}

pub(crate) async fn search(pool: &SqlitePool, filter: &UserFilter) -> sqlx::Result<Vec<UserView>> {
    list(pool, filter).await
}

/// Admin-only details that don't belong in the public user view.
#[derive(Debug, FromRow)]
pub(crate) struct UserExtra {
    pub login_count: i64,
    pub last_login_ip: Option<String>,
    pub status_reason: Option<String>,
    pub failed_attempts: Option<i64>,
    pub locked_until: Option<i64>,
    pub sessions: i64,
    pub api_keys: i64,
}

pub(crate) async fn extra(pool: &SqlitePool, user_id: i64) -> sqlx::Result<UserExtra> {
    sqlx::query_as(
        "SELECT u.login_count, u.last_login_ip, u.status_reason,
                c.failed_attempts, c.locked_until,
                (SELECT COUNT(*) FROM v_active_sessions s WHERE s.user_id = u.id) AS sessions,
                (SELECT COUNT(*) FROM api_keys k
                  WHERE k.user_id = u.id AND k.revoked_at IS NULL) AS api_keys
           FROM users u LEFT JOIN user_credentials c ON c.user_id = u.id
          WHERE u.id = ? AND u.deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
}
