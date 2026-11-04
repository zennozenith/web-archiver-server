//! Queries for captures: dated snapshots of a page, with their blobs and assets.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::{FromRow, QueryBuilder, Result, Sqlite, SqliteExecutor, SqlitePool};

use crate::{
    db::{
        content::ContentId,
        helper::{Cursor, CursorPage, group_by, id_type},
        page::{self, PageId},
    },
    hash::Blake3Hash,
    limit::Limit,
    mime::MimeType,
};

id_type! {
    /// Primary key of a capture.
    pub(crate) struct CaptureId;
}

/// Version numbers are computed (1 = oldest) so they renumber if a capture is deleted.
macro_rules! capture_summary_select {
    () => {
        "SELECT c.id AS id,
                ROW_NUMBER() OVER (ORDER BY c.captured_at, c.id) AS version,
                c.captured_at AS captured_at, c.title AS title, c.status_code AS status_code,
                NULLIF(lower(hex(h.hash)), '') AS content_hash,
                (c.screenshot_content_id IS NOT NULL) AS has_screenshot
         FROM capture c LEFT JOIN content h ON h.id = c.content_id
         WHERE c.page_id = ?"
    };
}

// ───────────────────────────────────────────────────────────────────────────
// Blobs
// ───────────────────────────────────────────────────────────────────────────

/// The content blobs a capture can reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureBlob {
    Html,
    OriginalHtml,
    Screenshot,
}

impl CaptureBlob {
    /// Column of `capture` holding this blob's content id.
    pub(crate) const fn column(self) -> &'static str {
        match self {
            Self::Html => "content_id",
            Self::OriginalHtml => "original_content_id",
            Self::Screenshot => "screenshot_content_id",
        }
    }

    /// Id of the page's newest capture that has this blob.
    pub(crate) async fn latest_capture_id<'e>(
        self,
        executor: impl SqliteExecutor<'e>,
        page_id: PageId,
    ) -> Result<Option<CaptureId>> {
        let mut qb: QueryBuilder<Sqlite> =
            QueryBuilder::new("SELECT id FROM capture WHERE page_id = ");
        qb.push_bind(page_id)
            .push(" AND ")
            .push(self.column()) // static column name, never user input
            .push(" IS NOT NULL ORDER BY captured_at DESC, id DESC LIMIT 1");

        qb.build_query_scalar().fetch_optional(executor).await
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Capture table operations
// ───────────────────────────────────────────────────────────────────────────

#[expect(dead_code, reason = "Might be needed later")]
pub(crate) async fn exists<'e>(executor: impl SqliteExecutor<'e>, id: CaptureId) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM capture WHERE id = ?)")
        .bind(id)
        .fetch_one(executor)
        .await
}

/// Id of the page's newest capture, if it has any.
pub(crate) async fn latest_id<'e>(
    executor: impl SqliteExecutor<'e>,
    page_id: PageId,
) -> Result<Option<CaptureId>> {
    sqlx::query_scalar(
        "SELECT id FROM capture WHERE page_id = ?
         ORDER BY captured_at DESC, id DESC LIMIT 1",
    )
    .bind(page_id)
    .fetch_optional(executor)
    .await
}

/// Returns `true` if a capture was deleted.
pub(crate) async fn delete<'e>(executor: impl SqliteExecutor<'e>, id: CaptureId) -> Result<bool> {
    let result = sqlx::query("DELETE FROM capture WHERE id = ?")
        .bind(id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() > 0)
}

// ───────────────────────────────────────────────────────────────────────────
// Overviews (index page)
// ───────────────────────────────────────────────────────────────────────────

/// One line per capture for the index UI.
#[derive(Debug, FromRow)]
pub(crate) struct CaptureOverview {
    pub id: CaptureId,
    pub page_id: PageId,
    pub captured_at: i64,
    pub title: Option<String>,
    pub status_code: Option<i64>,
    pub has_html: bool,
    pub has_screenshot: bool,
}

impl CaptureOverview {
    /// Captures for many pages in one query, newest first per page.
    pub(crate) async fn overviews_by_page<'e>(
        executor: impl SqliteExecutor<'e>,
        page_ids: &[PageId],
    ) -> Result<HashMap<PageId, Vec<Self>>> {
        if page_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT c.id, c.page_id, c.captured_at, c.title, c.status_code,
                (c.content_id IS NOT NULL) AS has_html,
                (c.screenshot_content_id IS NOT NULL) AS has_screenshot
         FROM capture c WHERE c.page_id IN (",
        );
        let mut list = qb.separated(", ");
        for &id in page_ids {
            list.push_bind(id);
        }
        list.push_unseparated(") ORDER BY c.page_id, c.captured_at DESC, c.id DESC");

        let rows: Vec<Self> = qb.build_query_as().fetch_all(executor).await?;
        Ok(group_by(rows.into_iter().map(|row| (row.page_id, row))))
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Summaries (API)
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureSummary {
    pub id: CaptureId,
    pub version: i64,
    pub captured_at: i64,
    pub title: Option<String>,
    pub status_code: Option<i64>,
    /// Hex-encoded content hash.
    pub content_hash: Option<String>,
    pub has_screenshot: bool,
}

impl CaptureSummary {
    /// Newest version first. `after` is the version of the last item already seen.
    pub(crate) async fn list(
        pool: &SqlitePool,
        page_id: PageId,
        after: Option<Cursor>,
        limit: Limit,
    ) -> Result<CursorPage<Self>> {
        let limit = i64::from(limit.get());

        let rows: Vec<Self> = sqlx::query_as(concat!(
            "SELECT * FROM (",
            capture_summary_select!(),
            ") WHERE version < ? ORDER BY version DESC LIMIT ?"
        ))
        .bind(page_id)
        .bind(after.map_or(i64::MAX, Cursor::get))
        .bind(limit.saturating_add(1)) // one extra row to detect another page
        .fetch_all(pool)
        .await?;

        Ok(CursorPage::from_overfetched(
            rows,
            usize::try_from(limit).unwrap_or(usize::MAX),
            |summary| Cursor::new(summary.version),
        ))
    }

    /// The page's newest capture, or `None` if it has none.
    pub(crate) async fn latest(pool: &SqlitePool, page_id: PageId) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "SELECT * FROM (",
            capture_summary_select!(),
            ") ORDER BY version DESC LIMIT 1"
        ))
        .bind(page_id)
        .fetch_optional(pool)
        .await
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Detail
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, FromRow)]
pub(crate) struct CaptureDetail {
    pub id: CaptureId,
    pub page_id: PageId,
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
    pub html_hash: Option<Blake3Hash>,
    pub html_size: Option<i64>,
    pub html_stored: Option<i64>,
    pub original_html_hash: Option<Blake3Hash>,
    pub original_html_size: Option<i64>,
    pub original_html_stored: Option<i64>,
    pub screenshot_hash: Option<Blake3Hash>,
    pub screenshot_size: Option<i64>,
    pub screenshot_mime: MimeType,
    pub asset_count: i64,
}

impl CaptureDetail {
    pub(crate) async fn load(pool: &SqlitePool, id: CaptureId) -> Result<Option<Self>> {
        sqlx::query_as(
            "SELECT c.id, c.page_id, p.url,
                    (SELECT COUNT(*) FROM capture c2
                      WHERE c2.page_id = c.page_id
                        AND (c2.captured_at, c2.id) <= (c.captured_at, c.id)) AS version,
                    c.captured_at, c.title, c.status_code, c.content_type, c.original_encoding,
                    c.request_headers, c.response_headers, c.metadata, c.error,
                    rp.url AS redirect_url,
                    NULLIF(lower(hex(h.hash)), '') AS html_hash,
                    h.size_original AS html_size, h.size_stored AS html_stored,
                    NULLIF(lower(hex(oh.hash)), '') AS original_html_hash,
                    oh.size_original AS original_html_size, oh.size_stored AS original_html_stored,
                    NULLIF(lower(hex(s.hash)), '') AS screenshot_hash,
                    s.size_original AS screenshot_size, s.mime_type AS screenshot_mime,
                    (SELECT COUNT(*) FROM capture_asset ca WHERE ca.capture_id = c.id) AS asset_count
             FROM capture c
             JOIN page p ON p.id = c.page_id
             LEFT JOIN page rp ON rp.id = c.redirect_page_id
             LEFT JOIN content h ON h.id = c.content_id
             LEFT JOIN content oh ON oh.id = c.original_content_id
             LEFT JOIN content s ON s.id = c.screenshot_content_id
             WHERE c.id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Assets
// ───────────────────────────────────────────────────────────────────────────

/// A sub-resource (image, stylesheet, ...) stored alongside a capture.
#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureAsset {
    pub content_id: ContentId,
    pub url: String,
    pub kind: String,
    pub status_code: Option<i64>,
    /// Hex-encoded content hash.
    pub hash: Blake3Hash,
    #[sqlx(try_from = "Option<String>")]
    pub mime_type: MimeType,
    pub size_original: i64,
    pub size_stored: i64,
}

impl CaptureAsset {
    pub(crate) async fn list<'e>(
        executor: impl SqliteExecutor<'e>,
        capture_id: CaptureId,
    ) -> Result<Vec<Self>> {
        sqlx::query_as(
            "SELECT ca.content_id, ca.url, ca.kind, ca.status_code,
                    lower(hex(ct.hash)) AS hash, ct.mime_type,
                    ct.size_original, ct.size_stored
             FROM capture_asset ca
             JOIN content ct ON ct.id = ca.content_id
             WHERE ca.capture_id = ?
             ORDER BY ca.kind, ca.url",
        )
        .bind(capture_id)
        .fetch_all(executor)
        .await
    }

    #[expect(dead_code, reason = "Might be needed later")]
    /// Assets of a capture; `None` if the capture does not exist.
    pub(crate) async fn for_capture(
        pool: &SqlitePool,
        capture_id: CaptureId,
    ) -> Result<Option<Vec<Self>>> {
        if !exists(pool, capture_id).await? {
            return Ok(None);
        }
        Ok(Some(Self::list(pool, capture_id).await?))
    }

    /// Assets of the page's newest capture (empty if it has none);
    /// `None` if the page does not exist.
    pub(crate) async fn for_latest_capture(
        pool: &SqlitePool,
        page_id: PageId,
    ) -> Result<Option<Vec<Self>>> {
        if !page::exists(pool, page_id).await? {
            return Ok(None);
        }
        let assets = match latest_id(pool, page_id).await? {
            Some(capture_id) => Self::list(pool, capture_id).await?,
            None => Vec::new(),
        };
        Ok(Some(assets))
    }
}
