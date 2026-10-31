use serde::Serialize;
use sqlx::{AssertSqlSafe, Result, SqlitePool, prelude::FromRow};

use crate::{db::contents::ContentRow, dto::Page};

pub(crate) const COLS: &str = "ct.id, ct.hash, ct.mime_type, ct.size_original, ct.size_stored, \
                        ct.compression_id, ct.data, ct.storage_ref, ct.ref_count, ct.created_at";

#[derive(Debug, FromRow)]
pub(crate) struct CaptureForPageRow {
    pub id: i64,
    pub url_id: i64,
    pub captured_at: i64,
    pub title: Option<String>,
    pub status_code: Option<i64>,
    pub has_html: bool,
    pub has_screenshot: bool,
}

#[derive(FromRow)]
pub(crate) struct AssetRow {
    pub content_id: i64,
    pub url: String,
    pub kind: String,
    pub status_code: Option<i64>,
    pub hash: String,
    pub mime_type: Option<String>,
    pub size_original: i64,
    pub size_stored: i64,
}

impl AssetRow {
    pub(crate) async fn list(pool: &SqlitePool, capture_id: i64) -> Result<Vec<Self>> {
        let rows: Vec<Self> = sqlx::query_as(
            "SELECT ca.content_id, ca.url, ca.kind, ca.status_code,
                lower(hex(ct.hash)) AS hash, ct.mime_type,
                ct.size_original, ct.size_stored
         FROM capture_asset ca
         JOIN content ct ON ct.id = ca.content_id
         WHERE ca.capture_id = ?
         ORDER BY ca.kind, ca.url",
        )
        .bind(capture_id)
        .fetch_all(pool)
        .await?;

        Ok(rows)
    }

    pub(crate) async fn capture_assets(
        pool: &SqlitePool,
        capture_id: i64,
    ) -> Result<Option<Vec<Self>>> {
        if sqlx::query_scalar::<_, i64>("SELECT id FROM capture WHERE id = ?")
            .bind(capture_id)
            .fetch_optional(pool)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(Self::list(pool, capture_id).await?))
    }

    pub(crate) async fn latest_assets(
        pool: &SqlitePool,
        page_id: i64,
    ) -> Result<Option<Vec<Self>>> {
        if sqlx::query_scalar::<_, i64>("SELECT id FROM url WHERE id = ?")
            .bind(page_id)
            .fetch_optional(pool)
            .await?
            .is_none()
        {
            return Ok(None);
        }

        let capture_id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM capture WHERE url_id = ?
         ORDER BY captured_at DESC, id DESC LIMIT 1",
        )
        .bind(page_id)
        .fetch_optional(pool)
        .await?;

        match capture_id {
            Some(capture_id) => Ok(Some(Self::list(pool, capture_id).await?)),
            None => Ok(Some(Vec::new())),
        }
    }

    pub(crate) async fn capture_asset(
        pool: &SqlitePool,
        capture_id: i64,
        content_id: i64,
    ) -> Result<Option<ContentRow>> {
        let row: Option<ContentRow> = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT {COLS}
         FROM capture_asset ca
         JOIN content ct ON ct.id = ca.content_id
         WHERE ca.capture_id = ? AND ca.content_id = ?
         LIMIT 1"
        )))
        .bind(capture_id)
        .bind(content_id)
        .fetch_optional(pool)
        .await?;

        Ok(row)
    }

    pub(crate) async fn where_not_null(
        pool: &SqlitePool,
        page_id: i64,
        col: &str, // fixed internal literal, never user input
    ) -> Result<Option<i64>> {
        let id = sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT id FROM capture WHERE url_id = ? AND {col} IS NOT NULL
         ORDER BY captured_at DESC, id DESC LIMIT 1"
        )))
        .bind(page_id)
        .fetch_optional(pool)
        .await?;

        Ok(id)
    }
}

#[derive(FromRow)]
pub(crate) struct CaptureDetailRow {
    pub id: i64,
    pub url_id: i64,
    pub url: String,
    pub version: i64,
    pub captured_at: i64,
    pub title: Option<String>,
    pub status_code: Option<i64>,
    pub content_type: Option<String>,
    pub original_encoding: Option<String>,
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    pub metadata: Option<String>,
    pub error: Option<String>,
    pub redirect_url: Option<String>,
    pub html_hash: Option<String>,
    pub html_size: Option<i64>,
    pub html_stored: Option<i64>,
    pub original_html_hash: Option<String>,
    pub original_html_size: Option<i64>,
    pub original_html_stored: Option<i64>,
    pub screenshot_hash: Option<String>,
    pub screenshot_size: Option<i64>,
    pub screenshot_mime: Option<String>,
    pub asset_count: i64,
}

impl CaptureDetailRow {
    pub(crate) async fn get(pool: &SqlitePool, capture_id: i64) -> Result<Option<Self>> {
        let row: Option<Self> = sqlx::query_as(
            "SELECT c.id, c.url_id, u.url,
                (SELECT COUNT(*) FROM capture c2
                  WHERE c2.url_id = c.url_id
                    AND (c2.captured_at, c2.id) <= (c.captured_at, c.id)) AS version,
                c.captured_at, c.title, c.status_code, c.content_type, c.original_encoding,
                c.request_headers, c.response_headers, c.metadata, c.error,
                ru.url AS redirect_url,
                NULLIF(lower(hex(h.hash)), '') AS html_hash,
                h.size_original AS html_size, h.size_stored AS html_stored,
                NULLIF(lower(hex(oh.hash)), '') AS original_html_hash,
                oh.size_original AS original_html_size, oh.size_stored AS original_html_stored,
                NULLIF(lower(hex(s.hash)), '') AS screenshot_hash,
                s.size_original AS screenshot_size, s.mime_type AS screenshot_mime,
                (SELECT COUNT(*) FROM capture_asset ca WHERE ca.capture_id = c.id) AS asset_count
         FROM capture c
         JOIN url u ON u.id = c.url_id
         LEFT JOIN url ru ON ru.id = c.redirect_url_id
         LEFT JOIN content h ON h.id = c.content_id
         LEFT JOIN content oh ON oh.id = c.original_content_id
         LEFT JOIN content s ON s.id = c.screenshot_content_id
         WHERE c.id = ?",
        )
        .bind(capture_id)
        .fetch_optional(pool)
        .await?;

        Ok(row)
    }

    pub(crate) async fn delete(pool: &SqlitePool, capture_id: i64) -> Result<u64> {
        let n = sqlx::query("DELETE FROM capture WHERE id = ?")
            .bind(capture_id)
            .execute(pool)
            .await?
            .rows_affected();
        Ok(n)
    }
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureSummary {
    pub id: i64,
    pub version: i64,
    pub captured_at: i64,
    pub title: Option<String>,
    pub status_code: Option<i64>,
    pub content_hash: Option<String>, // hex
    pub has_screenshot: bool,
}

/// Version numbers are computed (1 = oldest) so they renumber if a capture is deleted.
const CAPTURE_SUMMARY_SQL: &str = "SELECT c.id AS id,
    ROW_NUMBER() OVER (ORDER BY c.captured_at, c.id) AS version,
    c.captured_at AS captured_at, c.title AS title, c.status_code AS status_code,
    NULLIF(lower(hex(h.hash)), '') AS content_hash,
    (c.screenshot_content_id IS NOT NULL) AS has_screenshot
    FROM capture c LEFT JOIN content h ON h.id = c.content_id
    WHERE c.url_id = ?";

impl CaptureSummary {
    pub(crate) async fn list(
        pool: &SqlitePool,
        page_id: i64,
        cursor: Option<i64>,
        limit: u32,
    ) -> Result<Page<Self>> {
        let mut items: Vec<Self> = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT * FROM ({CAPTURE_SUMMARY_SQL}) WHERE version < ? ORDER BY version DESC LIMIT ?"
        )))
        .bind(page_id)
        .bind(cursor.unwrap_or(i64::MAX))
        .bind(limit.saturating_add(1))
        .fetch_all(pool)
        .await?;

        let has_more = i64::try_from(items.len()).map_or(true, |len| len > limit.into());
        items.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        let next_cursor = has_more
            .then(|| items.last().map(|c| c.version.to_string()))
            .flatten();

        Ok(Page { items, next_cursor })
    }

    pub(crate) async fn latest(pool: &SqlitePool, page_id: i64) -> Result<Self> {
        // Version numbers are computed (1 = oldest) so they renumber if a capture is deleted.
        let latest_capture = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT * FROM ({CAPTURE_SUMMARY_SQL}) ORDER BY version DESC LIMIT 1"
        )))
        .bind(page_id)
        .fetch_one(pool)
        .await?;

        Ok(latest_capture)
    }
}
