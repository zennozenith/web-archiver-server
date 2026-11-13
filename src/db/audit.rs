use serde_json::Value;
use sqlx::SqlitePool;

use crate::auth::AuditEntry;

/// Append a row to the append-only `audit_log`. Failures are logged, never bubbled up.
pub(crate) async fn audit(
    pool: &SqlitePool,
    actor: Option<i64>,
    target: Option<i64>,
    action: &str,
    details: Option<Value>,
) {
    let res = sqlx::query(
        "INSERT INTO audit_log (actor_user_id, target_user_id, action, details) VALUES (?, ?, ?, ?)",
    )
    .bind(actor)
    .bind(target)
    .bind(action)
    .bind(details.map(|v| v.to_string()))
    .execute(pool)
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
    sqlx::query_as(concat!(
        audit_select!(),
        " WHERE (?1 = '' OR a.action LIKE '%' || ?1 || '%'
                         OR actor.email LIKE '%' || ?1 || '%'
                         OR target.email LIKE '%' || ?1 || '%')
            AND (?2 IS NULL OR a.id < ?2)
          ORDER BY a.id DESC LIMIT ?3"
    ))
    .bind(q)
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
