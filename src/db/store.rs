//! Content-addressed blob storage with dedup.
//! Two phases so CPU work (hash, compress) happens outside the write transaction.

use sqlx::{SqliteConnection, SqlitePool};

use crate::blob_store::BlobStore;
use crate::compress;
use crate::error::{AppError, Result, join_err};
use crate::models::Compression;

pub(crate) enum Location {
    Inline(Vec<u8>),
    /// relative path, see `BlobStore`
    File(String),
}

pub(crate) struct NewPayload {
    pub location: Location,
    pub compression: Compression,
    pub size_stored: i64,
}

pub(crate) struct PreparedBlob {
    pub hash: Vec<u8>,
    pub mime: String,
    pub size_original: i64,
    /// None when the hash already existed (nothing to write).
    pub payload: Option<NewPayload>,
    pub existed: bool,
}

pub(crate) async fn prepare_blob(
    pool: &SqlitePool,
    store: &BlobStore,
    bytes: Vec<u8>,
    mime: &str,
) -> Result<PreparedBlob> {
    let mime = mime.to_owned();
    let size_original = i64::try_from(bytes.len()).unwrap_or(i64::MAX);

    let (hash, bytes) = tokio::task::spawn_blocking(move || {
        let h = blake3::hash(&bytes).as_bytes().to_vec();
        (h, bytes)
    })
    .await
    .map_err(join_err)?;

    let existed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM content WHERE hash = ?)")
        .bind(&hash)
        .fetch_one(pool)
        .await?;

    let payload = if existed {
        None
    } else {
        let m = mime.clone();
        let (stored, compression) =
            tokio::task::spawn_blocking(move || compress::compress_for_storage(&m, bytes))
                .await
                .map_err(join_err)??;

        let size_stored = i64::try_from(stored.len()).unwrap_or(i64::MAX);
        let location = if stored.len() > store.inline_threshold() {
            // Written BEFORE the DB row exists. If the transaction later fails we are left with
            // an unreferenced file, which is harmless and reusable by the next attempt.
            Location::File(store.put(&hash, stored).await?)
        } else {
            Location::Inline(stored)
        };
        Some(NewPayload {
            location,
            compression,
            size_stored,
        })
    };

    Ok(PreparedBlob {
        hash,
        mime,
        size_original,
        payload,
        existed,
    })
}

/// Returns the content id. Safe under races (ON CONFLICT DO NOTHING).
pub(crate) async fn insert_blob(conn: &mut SqliteConnection, b: &PreparedBlob) -> Result<i64> {
    if let Some(p) = &b.payload {
        let (data, storage_ref) = match &p.location {
            Location::Inline(d) => (Some(d.as_slice()), None),
            Location::File(r) => (None, Some(r.as_str())),
        };
        sqlx::query(
            "INSERT INTO content (hash, mime_type, size_original, size_stored, compression_id, data, storage_ref)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(hash) DO NOTHING",
        )
        .bind(&b.hash)
        .bind(&b.mime)
        .bind(b.size_original)
        .bind(p.size_stored)
        .bind(p.compression)
        .bind(data)
        .bind(storage_ref)
        .execute(&mut *conn)
        .await?;
    }

    sqlx::query_scalar("SELECT id FROM content WHERE hash = ?")
        .bind(&b.hash)
        .fetch_optional(&mut *conn)
        .await?
        // existed at prepare time but was garbage-collected before we got here
        .ok_or_else(|| AppError::Conflict("content was removed concurrently, please retry".into()))
}
