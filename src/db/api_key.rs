use sqlx::{Result, SqliteExecutor};

use crate::identity::ApiKeyView;

pub(crate) async fn list_for_user<'e>(
    ex: impl SqliteExecutor<'e>,
    user_id: i64,
) -> Result<Vec<ApiKeyView>> {
    sqlx::query_as(
        "SELECT public_id, name, key_prefix, scopes, created_at, last_used_at, expires_at
           FROM api_keys
          WHERE user_id = ? AND revoked_at IS NULL
          ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(ex)
    .await
}

pub(crate) struct InsertApiKey<'a> {
    pub user_id: i64,
    pub name: &'a str,
    pub prefix: &'a str,
    pub key_hash: &'a str,
    pub scopes_json: &'a str,
    pub expires_at: Option<i64>,
}

/// Returns the new key's `public_id`.
pub(crate) async fn insert<'e>(ex: impl SqliteExecutor<'e>, k: InsertApiKey<'_>) -> Result<String> {
    sqlx::query_scalar(
        "INSERT INTO api_keys (user_id, name, key_prefix, key_hash, scopes, expires_at)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING public_id",
    )
    .bind(k.user_id)
    .bind(k.name)
    .bind(k.prefix)
    .bind(k.key_hash)
    .bind(k.scopes_json)
    .bind(k.expires_at)
    .fetch_one(ex)
    .await
}

pub(crate) async fn revoke<'e>(
    ex: impl SqliteExecutor<'e>,
    user_id: i64,
    public_id: &str,
    now: i64,
) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE api_keys SET revoked_at = ?
          WHERE public_id = ? AND user_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(public_id)
    .bind(user_id)
    .execute(ex)
    .await?;
    Ok(res.rows_affected() > 0)
}
