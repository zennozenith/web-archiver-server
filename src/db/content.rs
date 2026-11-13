//! Queries for content-addressed blobs.

use sqlx::{FromRow, QueryBuilder, Result, Sqlite, SqliteExecutor};

use crate::{
    db::{
        capture::{CaptureBlob, CaptureId},
        helper::id_type,
        page::PageId,
    },
    infra::{Blake3Hash, Compression},
    util::mime::MimeType,
};

id_type! {
    /// Primary key of a stored content blob.
    pub(crate) struct ContentId;
}

/// Column list shared by every query that materializes a [`Content`].
macro_rules! content_columns {
    () => {
        "ct.id, ct.hash, ct.mime_type, ct.size_original, ct.size_stored, \
         ct.compression_id, ct.data, ct.storage_ref, ct.ref_count, ct.created_at"
    };
}

#[expect(
    dead_code,
    reason = "database row models mirror the schema; not every column is read yet"
)]
#[derive(Debug, FromRow)]
pub(crate) struct Content {
    pub id: ContentId,
    /// 32-byte BLAKE3 digest.
    pub hash: Vec<u8>,
    /// Defaults to `application/octet-stream` when the column is NULL.
    #[sqlx(try_from = "Option<String>")]
    pub mime_type: MimeType,
    pub size_original: i64,
    pub size_stored: i64,
    #[sqlx(rename = "compression_id")]
    pub compression: Compression,
    /// Inline bytes; `None` when the blob lives in external storage.
    pub data: Option<Vec<u8>>,
    pub storage_ref: Option<String>,
    pub ref_count: i64,
    pub created_at: i64,
}

/// Size and type of a blob, enough to answer a `HEAD` request.
#[derive(Debug, FromRow)]
pub(crate) struct ContentHead {
    pub size_original: i64,
    /// Defaults to `application/octet-stream` when the column is NULL.
    #[sqlx(try_from = "Option<String>")]
    pub mime_type: MimeType,
}

impl Content {
    pub(crate) async fn by_hash<'e>(
        executor: impl SqliteExecutor<'e>,
        hash: &Blake3Hash,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "SELECT ",
            content_columns!(),
            " FROM contents ct WHERE ct.hash = ?"
        ))
        .bind(hash)
        .fetch_optional(executor)
        .await
    }

    /// Whether the owner has a capture, page thumbnail or capture asset that
    /// references this blob.
    pub(crate) async fn visible_to<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        content_id: ContentId,
    ) -> Result<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM pages p
                 WHERE p.owner_user_id = ? AND p.thumbnail_content_id = ?
                UNION ALL
                SELECT 1 FROM captures c
                  JOIN pages p ON p.id = c.page_id
                 WHERE p.owner_user_id = ?
                   AND (c.content_id = ? OR c.original_content_id = ?
                        OR c.screenshot_content_id = ?)
                UNION ALL
                SELECT 1 FROM capture_assets ca
                  JOIN captures c ON c.id = ca.capture_id
                  JOIN pages p ON p.id = c.page_id
                 WHERE p.owner_user_id = ? AND ca.content_id = ?
             )",
        )
        .bind(owner)
        .bind(content_id)
        .bind(owner)
        .bind(content_id)
        .bind(content_id)
        .bind(content_id)
        .bind(owner)
        .bind(content_id)
        .fetch_one(executor)
        .await
    }

    /// The page's thumbnail blob.
    pub(crate) async fn for_page<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        page_id: PageId,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "SELECT ",
            content_columns!(),
            " FROM pages p
              JOIN contents ct ON ct.id = p.thumbnail_content_id
              WHERE p.owner_user_id = ? AND p.id = ?
              LIMIT 1"
        ))
        .bind(owner)
        .bind(page_id)
        .fetch_optional(executor)
        .await
    }

    /// The blob of the given kind attached to a capture.
    pub(crate) async fn for_capture<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        capture_id: CaptureId,
        blob: CaptureBlob,
    ) -> Result<Option<Self>> {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(concat!(
            "SELECT ",
            content_columns!(),
            " FROM captures c
              JOIN pages p ON p.id = c.page_id
              JOIN contents ct ON ct.id = c."
        ));
        qb.push(blob.column()) // static column name, never user input
            .push(" WHERE p.owner_user_id = ")
            .push_bind(owner)
            .push(" AND c.id = ")
            .push_bind(capture_id);

        qb.build_query_as().fetch_optional(executor).await
    }

    /// A specific asset blob of a capture.
    pub(crate) async fn for_capture_asset<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        capture_id: CaptureId,
        content_id: ContentId,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "SELECT ",
            content_columns!(),
            " FROM capture_assets ca
              JOIN captures c ON c.id = ca.capture_id
              JOIN pages p ON p.id = c.page_id
              JOIN contents ct ON ct.id = ca.content_id
              WHERE p.owner_user_id = ? AND ca.capture_id = ? AND ca.content_id = ?
              LIMIT 1"
        ))
        .bind(owner)
        .bind(capture_id)
        .bind(content_id)
        .fetch_optional(executor)
        .await
    }
}

impl ContentHead {
    pub(crate) async fn by_hash<'e>(
        executor: impl SqliteExecutor<'e>,
        hash: &Blake3Hash,
    ) -> Result<Option<Self>> {
        sqlx::query_as("SELECT size_original, mime_type FROM contents WHERE hash = ?")
            .bind(hash)
            .fetch_optional(executor)
            .await
    }

    /// Like [`Self::by_hash`], but only when `owner` references the blob.
    pub(crate) async fn by_hash_visible<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        hash: &Blake3Hash,
    ) -> Result<Option<Self>> {
        sqlx::query_as(
            "SELECT ct.size_original, ct.mime_type
               FROM contents ct
              WHERE ct.hash = ?
                AND EXISTS (
                    SELECT 1 FROM pages p
                     WHERE p.owner_user_id = ? AND p.thumbnail_content_id = ct.id
                    UNION ALL
                    SELECT 1 FROM captures c
                      JOIN pages p ON p.id = c.page_id
                     WHERE p.owner_user_id = ?
                       AND (c.content_id = ct.id OR c.original_content_id = ct.id
                            OR c.screenshot_content_id = ct.id)
                    UNION ALL
                    SELECT 1 FROM capture_assets ca
                      JOIN captures c ON c.id = ca.capture_id
                      JOIN pages p ON p.id = c.page_id
                     WHERE p.owner_user_id = ? AND ca.content_id = ct.id
                )",
        )
        .bind(hash)
        .bind(owner)
        .bind(owner)
        .bind(owner)
        .fetch_optional(executor)
        .await
    }
}
