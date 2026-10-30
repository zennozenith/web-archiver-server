use std::time::Duration;

use axum::extract::State;
use sqlx::SqlitePool;

use crate::{
    blob_store::BlobStore,
    db::stats::{self, Stats},
    error::Result,
    extract::ApiJson,
};

// GET /api/health  (unauthenticated)
pub(super) async fn health(State(pool): State<SqlitePool>) -> Result<ApiJson<serde_json::Value>> {
    sqlx::query("SELECT 1").execute(&pool).await?;
    Ok(ApiJson(serde_json::json!({ "status": "ok" })))
}

// GET /api/stats
pub(super) async fn stats(State(pool): State<SqlitePool>) -> Result<ApiJson<Stats>> {
    let stat = stats::stats(&pool).await?;
    Ok(ApiJson(stat))
}

// POST /api/maintenance/gc
// 1. deletes content rows no capture references (1h grace avoids racing in-flight archives)
// 2. deletes the files those rows pointed at
// 3. removes abandoned temp files from crashed writes
//
// ref_count is maintained by triggers on capture/capture_asset, so it covers the
// document, screenshot and every saved sub-resource.
//
// DB first, files second: a crash in between leaves an orphan file, never a row pointing at nothing.
pub(super) async fn gc_content(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
) -> Result<ApiJson<serde_json::Value>> {
    let refs: Vec<Option<String>> = sqlx::query_scalar(
        "DELETE FROM content
         WHERE created_at < unixepoch() - 3600
           AND ref_count <= 0
         RETURNING storage_ref",
    )
    .fetch_all(&pool)
    .await?;

    let rows_deleted = refs.len();
    let mut files_deleted: usize = 0;
    for r in refs.into_iter().flatten() {
        // Skip if the same hash was re-archived (new row, same path) while we were deleting.
        let reused: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM content WHERE storage_ref = ?)")
                .bind(&r)
                .fetch_one(&pool)
                .await?;
        if !reused {
            store.remove(&r).await?;
            files_deleted = files_deleted.saturating_add(1);
        }
    }
    let tmp_removed = store.clean_tmp(Duration::from_secs(3600)).await;

    Ok(ApiJson(serde_json::json!({
        "rows_deleted": rows_deleted,
        "files_deleted": files_deleted,
        "tmp_removed": tmp_removed,
    })))
}
