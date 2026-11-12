use std::{collections::BTreeMap, time::Duration};

use serde::Serialize;
use sqlx::{Result, SqlitePool, prelude::FromRow};

use crate::{infra::BlobStore, prelude::*};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GCStat {
    rows_deleted: usize,
    files_deleted: usize,
    tmp_removed: usize,
}

#[derive(FromRow)]
struct Totals {
    pages: i64,
    captures: i64,
    content_items: i64,
    assets: i64,
    tags: i64,
    pendings: i64,
    stored_bytes: i64,
    unique_original_bytes: i64,
    logical_bytes: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Stats {
    pub pages: i64,
    pub captures: i64,
    pub content_items: i64,
    pub assets: i64,
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
pub(crate) async fn stats(pool: &SqlitePool) -> Result<Stats> {
    let t: Totals = sqlx::query_as(
        "SELECT
           (SELECT COUNT(*) FROM page)    AS pages,
           (SELECT COUNT(*) FROM capture) AS captures,
           (SELECT COUNT(*) FROM content) AS content_items,
           (SELECT COUNT(*) FROM capture_asset) AS assets,
           (SELECT COUNT(*) FROM tag) AS tags,
           (SELECT COUNT(*) FROM pending) AS pendings,
           COALESCE((SELECT SUM(size_stored)   FROM content), 0) AS stored_bytes,
           COALESCE((SELECT SUM(size_original) FROM content), 0) AS unique_original_bytes,
           COALESCE((SELECT SUM(ct.size_original) FROM capture c JOIN content ct ON ct.id = c.content_id), 0)
         + COALESCE((SELECT SUM(ct.size_original) FROM capture c JOIN content ct ON ct.id = c.original_content_id
                     WHERE c.original_content_id IS NOT c.content_id), 0)
         + COALESCE((SELECT SUM(ct.size_original) FROM capture c JOIN content ct ON ct.id = c.screenshot_content_id), 0)
         + COALESCE((SELECT SUM(ct.size_original) FROM capture_asset ca JOIN content ct ON ct.id = ca.content_id), 0)
           AS logical_bytes",
    )
    .fetch_one(pool)
    .await?;

    let mut queue: BTreeMap<String, i64> = ["queued", "in_progress", "failed"]
        .into_iter()
        .map(|s| (s.to_owned(), 0))
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
        assets: t.assets,
        tags: t.tags,
        pendings: t.pendings,
        stored_bytes: t.stored_bytes,
        unique_original_bytes: t.unique_original_bytes,
        logical_bytes: t.logical_bytes,
        saved_bytes: t.logical_bytes.saturating_sub(t.stored_bytes),
        queue,
    })
}

pub(crate) async fn health(pool: &SqlitePool) -> Result<()> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

// 1. deletes content rows no capture references (1h grace avoids racing in-flight archives)
// 2. deletes the files those rows pointed at
// 3. removes abandoned temp files from crashed writes
//
// ref_count is maintained by triggers on capture/capture_asset, so it covers the
// document, screenshot and every saved sub-resource.
//
// DB first, files second: a crash in between leaves an orphan file, never a row pointing at nothing.
pub(crate) async fn gc_content(pool: &SqlitePool, store: &BlobStore) -> Result<GCStat, AppError> {
    let refs: Vec<Option<String>> = sqlx::query_scalar(
        "DELETE FROM content
         WHERE created_at < unixepoch() - 3600
           AND ref_count <= 0
         RETURNING storage_ref",
    )
    .fetch_all(pool)
    .await?;

    let rows_deleted = refs.len();
    let mut files_deleted: usize = 0;
    for r in refs.into_iter().flatten() {
        // Skip if the same hash was re-archived (new row, same path) while we were deleting.
        let reused: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM content WHERE storage_ref = ?)")
                .bind(&r)
                .fetch_one(pool)
                .await?;
        if !reused {
            store.remove(&r).await?;
            files_deleted = files_deleted.saturating_add(1);
        }
    }
    let tmp_removed = store.clean_tmp(Duration::from_secs(3600)).await;

    Ok(GCStat {
        rows_deleted,
        files_deleted,
        tmp_removed,
    })
}
