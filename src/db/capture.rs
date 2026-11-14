//! Queries for captures: dated snapshots of a page, with their blobs and assets.

use serde::Serialize;
use sqlx::{FromRow, QueryBuilder, Result, Sqlite, SqliteConnection, SqliteExecutor, SqlitePool};

use crate::{
    db::{
        content::ContentId,
        cursor::{Cursor, CursorPage},
        ids::id_type,
        page::{self, PageId},
    },
    infra::Blake3Hash,
    util::{limit::Limit, mime::MimeType},
};

id_type! {
    /// Primary key of a capture.
    pub(crate) struct CaptureId;
}

impl CaptureId {
    pub(crate) const fn get(self) -> i64 {
        self.0
    }
}

/// Version numbers are computed (1 = oldest) so they renumber if a capture is deleted.
macro_rules! capture_summary_select {
    () => {
        "SELECT c.id AS id,
                ROW_NUMBER() OVER (ORDER BY c.captured_at, c.id) AS version,
                c.captured_at AS captured_at, c.title AS title, c.status_code AS status_code,
                NULLIF(lower(hex(h.hash)), '') AS content_hash,
                (c.screenshot_content_id IS NOT NULL) AS has_screenshot
         FROM captures c
         JOIN pages p ON p.id = c.page_id
         LEFT JOIN contents h ON h.id = c.content_id
         WHERE p.owner_user_id = ? AND c.page_id = ?"
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
    /// Column of `captures` holding this blob's content id.
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
        owner: i64,
        page_id: PageId,
    ) -> Result<Option<CaptureId>> {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT c.id FROM captures c JOIN pages p ON p.id = c.page_id
             WHERE p.owner_user_id = ",
        );
        qb.push_bind(owner)
            .push(" AND c.page_id = ")
            .push_bind(page_id)
            .push(" AND c.")
            .push(self.column()) // static column name, never user input
            .push(" IS NOT NULL ORDER BY c.captured_at DESC, c.id DESC LIMIT 1");

        qb.build_query_scalar().fetch_optional(executor).await
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Capture table operations
// ───────────────────────────────────────────────────────────────────────────

#[expect(dead_code, reason = "Might be needed later")]
pub(crate) async fn exists<'e>(
    executor: impl SqliteExecutor<'e>,
    owner: i64,
    id: CaptureId,
) -> Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM captures c JOIN pages p ON p.id = c.page_id
            WHERE p.owner_user_id = ? AND c.id = ?
        )",
    )
    .bind(owner)
    .bind(id)
    .fetch_one(executor)
    .await
}

/// Id of the page's newest capture, if it has any.
pub(crate) async fn latest_id<'e>(
    executor: impl SqliteExecutor<'e>,
    owner: i64,
    page_id: PageId,
) -> Result<Option<CaptureId>> {
    sqlx::query_scalar(
        "SELECT c.id FROM captures c JOIN pages p ON p.id = c.page_id
         WHERE p.owner_user_id = ? AND c.page_id = ?
         ORDER BY c.captured_at DESC, c.id DESC LIMIT 1",
    )
    .bind(owner)
    .bind(page_id)
    .fetch_optional(executor)
    .await
}

/// Returns `true` if a capture was deleted.
pub(crate) async fn delete<'e>(
    executor: impl SqliteExecutor<'e>,
    owner: i64,
    id: CaptureId,
) -> Result<bool> {
    let result = sqlx::query(
        "DELETE FROM captures
         WHERE id = ? AND page_id IN (SELECT id FROM pages WHERE owner_user_id = ?)",
    )
    .bind(id)
    .bind(owner)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ───────────────────────────────────────────────────────────────────────────
// Capture creation
// ───────────────────────────────────────────────────────────────────────────

/// Everything needed to insert a capture row.
pub(crate) struct NewCapture<'a> {
    pub owner: i64,
    pub page_id: PageId,
    pub content_id: ContentId,
    pub original_content_id: ContentId,
    pub screenshot_content_id: Option<ContentId>,
    pub captured_at: i64,
    pub title: Option<&'a str>,
    pub content_type: &'a str,
    pub metadata: Option<&'a str>,
}

impl NewCapture<'_> {
    /// Inserts the capture for the owner's page and returns its id and version.
    pub(crate) async fn insert(&self, conn: &mut SqliteConnection) -> Result<(CaptureId, i64)> {
        let capture_id: CaptureId = sqlx::query_scalar(
            "INSERT INTO captures (page_id, content_id, original_content_id,
                                   screenshot_content_id, captured_at, content_type,
                                   metadata, title)
             SELECT ?, ?, ?, ?, ?, ?, ?, ? FROM pages
              WHERE id = ? AND owner_user_id = ?
             RETURNING id",
        )
        .bind(self.page_id)
        .bind(self.content_id)
        .bind(self.original_content_id)
        .bind(self.screenshot_content_id)
        .bind(self.captured_at)
        .bind(self.content_type)
        .bind(self.metadata)
        .bind(self.title.unwrap_or(""))
        .bind(self.page_id)
        .bind(self.owner)
        .fetch_one(&mut *conn)
        .await?;

        let version: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM captures WHERE page_id = ?")
            .bind(self.page_id)
            .fetch_one(&mut *conn)
            .await?;

        Ok((capture_id, version))
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
        owner: i64,
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
        .bind(owner)
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
    pub(crate) async fn latest(
        pool: &SqlitePool,
        owner: i64,
        page_id: PageId,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "SELECT * FROM (",
            capture_summary_select!(),
            ") ORDER BY version DESC LIMIT 1"
        ))
        .bind(owner)
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
    pub screenshot_stored: Option<i64>,
    #[sqlx(try_from = "Option<String>")]
    pub screenshot_mime: MimeType,
    pub asset_count: i64,
}

impl CaptureDetail {
    pub(crate) async fn load(pool: &SqlitePool, owner: i64, id: CaptureId) -> Result<Option<Self>> {
        sqlx::query_as(
            "SELECT c.id, c.page_id, p.url,
                    (SELECT COUNT(*) FROM captures c2
                      WHERE c2.page_id = c.page_id
                        AND (c2.captured_at, c2.id) <= (c.captured_at, c.id)) AS version,
                    c.captured_at, c.title, c.status_code, c.content_type, c.original_encoding,
                    c.request_headers, c.response_headers, c.metadata, c.error,
                    rp.url AS redirect_url,
                    h.hash AS html_hash,
                    h.size_original AS html_size, h.size_stored AS html_stored,
                    oh.hash AS original_html_hash,
                    oh.size_original AS original_html_size, oh.size_stored AS original_html_stored,
                    s.hash AS screenshot_hash,
                    s.size_original AS screenshot_size, s.size_stored AS screenshot_stored,
                    s.mime_type AS screenshot_mime,
                    (SELECT COUNT(*) FROM capture_assets ca WHERE ca.capture_id = c.id) AS asset_count
             FROM captures c
             JOIN pages p ON p.id = c.page_id
             LEFT JOIN pages rp ON rp.id = c.redirect_page_id
             LEFT JOIN contents h ON h.id = c.content_id
             LEFT JOIN contents oh ON oh.id = c.original_content_id
             LEFT JOIN contents s ON s.id = c.screenshot_content_id
             WHERE p.owner_user_id = ? AND c.id = ?",
        )
        .bind(owner)
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
        owner: i64,
        capture_id: CaptureId,
    ) -> Result<Vec<Self>> {
        sqlx::query_as(
            "SELECT ca.content_id, ca.url, ca.kind, ca.status_code,
                    lower(hex(ct.hash)) AS hash, ct.mime_type,
                    ct.size_original, ct.size_stored
             FROM capture_assets ca
             JOIN captures c ON c.id = ca.capture_id
             JOIN pages p ON p.id = c.page_id
             JOIN contents ct ON ct.id = ca.content_id
             WHERE p.owner_user_id = ? AND ca.capture_id = ?
             ORDER BY ca.kind, ca.url",
        )
        .bind(owner)
        .bind(capture_id)
        .fetch_all(executor)
        .await
    }

    #[expect(dead_code, reason = "Might be needed later")]
    /// Assets of a capture; `None` if the capture does not exist.
    pub(crate) async fn for_capture(
        pool: &SqlitePool,
        owner: i64,
        capture_id: CaptureId,
    ) -> Result<Option<Vec<Self>>> {
        if !exists(pool, owner, capture_id).await? {
            return Ok(None);
        }
        Ok(Some(Self::list(pool, owner, capture_id).await?))
    }

    /// Assets of the page's newest capture (empty if it has none);
    /// `None` if the page does not exist.
    pub(crate) async fn for_latest_capture(
        pool: &SqlitePool,
        owner: i64,
        page_id: PageId,
    ) -> Result<Option<Vec<Self>>> {
        if !page::exists(pool, owner, page_id).await? {
            return Ok(None);
        }
        let assets = match latest_id(pool, owner, page_id).await? {
            Some(capture_id) => Self::list(pool, owner, capture_id).await?,
            None => Vec::new(),
        };
        Ok(Some(assets))
    }
}
