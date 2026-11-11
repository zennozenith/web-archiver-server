use serde_json::Value;
use sqlx::SqlitePool;

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
