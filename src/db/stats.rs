use std::{collections::BTreeMap, time::Duration};

use chrono::{Days, NaiveDate};
use serde::Serialize;
use sqlx::{FromRow, Result, SqlitePool};

use crate::{
    infra::BlobFiles,
    prelude::*,
    util::time::{DAY, HOUR, days_ms, duration_ms, now_ms, today},
};

#[derive(Serialize)]
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
           (SELECT COUNT(*) FROM pages)    AS pages,
           (SELECT COUNT(*) FROM captures) AS captures,
           (SELECT COUNT(*) FROM contents) AS content_items,
           (SELECT COUNT(*) FROM capture_assets) AS assets,
           (SELECT COUNT(*) FROM tags) AS tags,
           (SELECT COUNT(*) FROM pending_items) AS pendings,
           COALESCE((SELECT SUM(size_stored)   FROM contents), 0) AS stored_bytes,
           COALESCE((SELECT SUM(size_original) FROM contents), 0) AS unique_original_bytes,
           COALESCE((SELECT SUM(ct.size_original) FROM captures c JOIN contents ct ON ct.id = c.content_id), 0)
         + COALESCE((SELECT SUM(ct.size_original) FROM captures c JOIN contents ct ON ct.id = c.original_content_id
                     WHERE c.original_content_id IS NOT c.content_id), 0)
         + COALESCE((SELECT SUM(ct.size_original) FROM captures c JOIN contents ct ON ct.id = c.screenshot_content_id), 0)
         + COALESCE((SELECT SUM(ct.size_original) FROM capture_assets ca JOIN contents ct ON ct.id = ca.content_id), 0)
           AS logical_bytes",
    )
    .fetch_one(pool)
    .await?;

    let mut queue: BTreeMap<String, i64> = ["queued", "in_progress", "failed"]
        .into_iter()
        .map(|s| (s.to_owned(), 0))
        .collect();
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM pending_items GROUP BY status")
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
pub(crate) async fn gc_content(pool: &SqlitePool, store: &BlobFiles) -> Result<GCStat, AppError> {
    let cutoff = now_ms().saturating_sub(duration_ms(HOUR));
    let refs: Vec<Option<String>> = sqlx::query_scalar(
        "DELETE FROM contents
         WHERE created_at < ?
           AND ref_count <= 0
         RETURNING storage_ref",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;

    let rows_deleted = refs.len();
    let mut files_deleted: usize = 0;
    for r in refs.into_iter().flatten() {
        // Skip if the same hash was re-archived (new row, same path) while we were deleting.
        let reused: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM contents WHERE storage_ref = ?)")
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

#[derive(Debug, FromRow)]
pub(crate) struct UserCounts {
    pub total: i64,
    pub new_week: i64,
    pub unverified: i64,
    pub active: i64,
    pub pending: i64,
    pub suspended: i64,
    pub deactivated: i64,
}

pub(crate) async fn user_counts(pool: &SqlitePool, now: i64) -> sqlx::Result<UserCounts> {
    sqlx::query_as(
        "SELECT COUNT(*) AS total,
                COALESCE(SUM(created_at >= ?), 0)       AS new_week,
                COALESCE(SUM(email IS NOT NULL AND email_verified_at IS NULL), 0) AS unverified,
                COALESCE(SUM(status = 'active'), 0)      AS active,
                COALESCE(SUM(status = 'pending'), 0)     AS pending,
                COALESCE(SUM(status = 'suspended'), 0)   AS suspended,
                COALESCE(SUM(status = 'deactivated'), 0) AS deactivated
           FROM users WHERE deleted_at IS NULL",
    )
    .bind(now.saturating_sub(days_ms(7)))
    .fetch_one(pool)
    .await
}

#[derive(Debug, FromRow)]
pub(crate) struct SecurityCounts {
    pub active_sessions: i64,
    pub failed_day: i64,
    pub locked: i64,
}

pub(crate) async fn security_counts(pool: &SqlitePool, now: i64) -> sqlx::Result<SecurityCounts> {
    sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM v_active_sessions) AS active_sessions,
                (SELECT COUNT(*) FROM login_attempts
                  WHERE success = 0 AND created_at >= ?1) AS failed_day,
                (SELECT COUNT(*) FROM user_credentials c JOIN users u ON u.id = c.user_id
                  WHERE u.deleted_at IS NULL AND c.locked_until > ?2) AS locked",
    )
    .bind(now.saturating_sub(duration_ms(DAY)))
    .bind(now)
    .fetch_one(pool)
    .await
}

#[derive(Debug)]
pub(crate) struct DayCount {
    pub date: NaiveDate,
    pub ok: i64,
    pub failed: i64,
}

/// Sign-ins per day for the last seven days (UTC), oldest first, zero-filled.
pub(crate) async fn login_days(pool: &SqlitePool) -> sqlx::Result<Vec<DayCount>> {
    let today = today();
    let days: Vec<NaiveDate> = (0..7u64)
        .rev()
        .filter_map(|n| today.checked_sub_days(Days::new(n)))
        .collect();
    let since = days
        .first()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map_or(0, |dt| dt.and_utc().timestamp_millis());

    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT date(created_at / 1000, 'unixepoch'),
                COALESCE(SUM(success), 0), COALESCE(SUM(1 - success), 0)
           FROM login_attempts WHERE created_at >= ? GROUP BY 1",
    )
    .bind(since)
    .fetch_all(pool)
    .await?;

    Ok(days
        .into_iter()
        .map(|date| {
            let key = date.format("%Y-%m-%d").to_string();
            let (ok, failed) = rows
                .iter()
                .find(|(d, ..)| *d == key)
                .map_or((0, 0), |&(_, ok, failed)| (ok, failed));
            DayCount { date, ok, failed }
        })
        .collect())
}
