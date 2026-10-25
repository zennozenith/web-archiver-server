use std::collections::BTreeMap;

use axum::extract::State;
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

use crate::{error::Result, extract::ApiJson};

// GET /api/health  (unauthenticated)
pub async fn health(State(pool): State<SqlitePool>) -> Result<ApiJson<serde_json::Value>> {
    sqlx::query("SELECT 1").execute(&pool).await?;
    Ok(ApiJson(serde_json::json!({ "status": "ok" })))
}

#[derive(FromRow)]
struct Totals {
    pages: i64,
    captures: i64,
    content_items: i64,
    stored_bytes: i64,
    unique_original_bytes: i64,
    logical_bytes: i64,
}

#[derive(Serialize)]
pub struct Stats {
    pages: i64,
    captures: i64,
    content_items: i64,
    /// bytes actually on disk (compressed, deduplicated)
    stored_bytes: i64,
    /// uncompressed size of unique content
    unique_original_bytes: i64,
    /// what you'd need without dedup or compression
    logical_bytes: i64,
    saved_bytes: i64,
    queue: BTreeMap<String, i64>,
}

// GET /api/stats
pub async fn stats(State(pool): State<SqlitePool>) -> Result<ApiJson<Stats>> {
    let t: Totals = sqlx::query_as(
        "SELECT
           (SELECT COUNT(*) FROM url)     AS pages,
           (SELECT COUNT(*) FROM capture) AS captures,
           (SELECT COUNT(*) FROM content) AS content_items,
           COALESCE((SELECT SUM(size_stored)   FROM content), 0) AS stored_bytes,
           COALESCE((SELECT SUM(size_original) FROM content), 0) AS unique_original_bytes,
           COALESCE((SELECT SUM(ct.size_original) FROM capture c JOIN content ct ON ct.id = c.content_id), 0)
         + COALESCE((SELECT SUM(ct.size_original) FROM capture c JOIN content ct ON ct.id = c.screenshot_content_id), 0)
           AS logical_bytes",
    )
    .fetch_one(&pool)
    .await?;

    let mut queue: BTreeMap<String, i64> = ["queued", "in_progress", "failed"]
        .into_iter()
        .map(|s| (s.to_string(), 0))
        .collect();
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM pending GROUP BY status")
            .fetch_all(&pool)
            .await?;
    queue.extend(rows);

    Ok(ApiJson(Stats {
        pages: t.pages,
        captures: t.captures,
        content_items: t.content_items,
        stored_bytes: t.stored_bytes,
        unique_original_bytes: t.unique_original_bytes,
        logical_bytes: t.logical_bytes,
        saved_bytes: t.logical_bytes - t.stored_bytes,
        queue,
    }))
}

// POST /api/maintenance/gc  -> deletes content no capture references.
// The 1h grace period avoids racing with in-flight archive requests.
pub async fn gc_content(State(pool): State<SqlitePool>) -> Result<ApiJson<serde_json::Value>> {
    let n = sqlx::query(
        "DELETE FROM content
         WHERE created_at < unixepoch() - 3600
           AND id NOT IN (SELECT content_id FROM capture WHERE content_id IS NOT NULL
                          UNION
                          SELECT screenshot_content_id FROM capture WHERE screenshot_content_id IS NOT NULL)",
    )
    .execute(&pool)
    .await?
    .rows_affected();
    Ok(ApiJson(serde_json::json!({ "deleted": n })))
}
