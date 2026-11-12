use sqlx::{Result, SqliteExecutor};

use crate::auth::SessionView;

pub(crate) async fn list_for_user<'e>(
    ex: impl SqliteExecutor<'e>,
    user_id: i64,
) -> Result<Vec<SessionView>> {
    sqlx::query_as(
        "SELECT public_id, ip_address, user_agent, created_at, last_used_at, expires_at
           FROM v_active_sessions
          WHERE user_id = ?
          ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(ex)
    .await
}

/// Only the owner's own live session can be revoked. `false` = nothing matched.
pub(crate) async fn revoke<'e>(
    ex: impl SqliteExecutor<'e>,
    user_id: i64,
    public_id: &str,
    now: i64,
) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE sessions SET revoked_at = ?, revoked_reason = 'user_revoked'
          WHERE public_id = ? AND user_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(public_id)
    .bind(user_id)
    .execute(ex)
    .await?;
    Ok(res.rows_affected() > 0)
}
