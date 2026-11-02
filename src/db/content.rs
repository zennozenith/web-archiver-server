//! Queries for content-addressed blobs.

use sqlx::{FromRow, QueryBuilder, Result, Sqlite, SqliteExecutor};

use crate::{
    compress::Compression,
    db::{
        capture::{CaptureBlob, CaptureId},
        helper::id_type,
    },
    hash::Blake3Hash,
    mime::MimeType,
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
            " FROM content ct WHERE ct.hash = ?"
        ))
        .bind(hash)
        .fetch_optional(executor)
        .await
    }

    /// The blob of the given kind attached to a capture.
    pub(crate) async fn for_capture<'e>(
        executor: impl SqliteExecutor<'e>,
        capture_id: CaptureId,
        blob: CaptureBlob,
    ) -> Result<Option<Self>> {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(concat!(
            "SELECT ",
            content_columns!(),
            " FROM capture c JOIN content ct ON ct.id = c."
        ));
        qb.push(blob.column()) // static column name, never user input
            .push(" WHERE c.id = ")
            .push_bind(capture_id);

        qb.build_query_as().fetch_optional(executor).await
    }

    /// A specific asset blob of a capture.
    pub(crate) async fn for_capture_asset<'e>(
        executor: impl SqliteExecutor<'e>,
        capture_id: CaptureId,
        content_id: ContentId,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "SELECT ",
            content_columns!(),
            " FROM capture_asset ca
              JOIN content ct ON ct.id = ca.content_id
              WHERE ca.capture_id = ? AND ca.content_id = ?
              LIMIT 1"
        ))
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
        sqlx::query_as("SELECT size_original, mime_type FROM content WHERE hash = ?")
            .bind(hash)
            .fetch_optional(executor)
            .await
    }
}
