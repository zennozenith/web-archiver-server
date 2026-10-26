use std::collections::BTreeMap;

use serde::Serialize;
use sqlx::{SqlitePool, prelude::FromRow};

#[derive(FromRow)]
struct Totals {
    pages: i64,
    captures: i64,
    content_items: i64,
    tags: i64,
    pendings: i64,
    stored_bytes: i64,
    unique_original_bytes: i64,
    logical_bytes: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub pages: i64,
    pub captures: i64,
    pub content_items: i64,
    pub tags: i64,
    pub pendings: i64,
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
pub async fn stats(pool: &SqlitePool) -> Result<Stats, sqlx::Error> {
    let t: Totals = sqlx::query_as(
        "SELECT
           (SELECT COUNT(*) FROM url)     AS pages,
           (SELECT COUNT(*) FROM capture) AS captures,
           (SELECT COUNT(*) FROM content) AS content_items,
           (SELECT COUNT(*) FROM tag) AS tags,
           (SELECT COUNT(*) FROM pending) AS pendings,
           COALESCE((SELECT SUM(size_stored)   FROM content), 0) AS stored_bytes,
           COALESCE((SELECT SUM(size_original) FROM content), 0) AS unique_original_bytes,
           COALESCE((SELECT SUM(ct.size_original) FROM capture c JOIN content ct ON ct.id = c.content_id), 0)
         + COALESCE((SELECT SUM(ct.size_original) FROM capture c JOIN content ct ON ct.id = c.screenshot_content_id), 0)
           AS logical_bytes",
    )
    .fetch_one(pool)
    .await?;

    let mut queue: BTreeMap<String, i64> = ["queued", "in_progress", "failed"]
        .into_iter()
        .map(|s| (s.to_string(), 0))
        .collect();
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM pending GROUP BY status")
            .fetch_all(pool)
            .await?;
    queue.extend(rows);

    Ok(Stats {
        pages: t.pages,
        captures: t.captures,
        content_items: t.content_items,
        tags: t.tags,
        pendings: t.pendings,
        stored_bytes: t.stored_bytes,
        unique_original_bytes: t.unique_original_bytes,
        logical_bytes: t.logical_bytes,
        saved_bytes: t.logical_bytes - t.stored_bytes,
        queue,
    })
}
