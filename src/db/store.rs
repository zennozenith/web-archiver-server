//! Content-addressed blob storage with dedup.
//! Two phases so CPU work (hash, compress) happens outside the write transaction.

use sqlx::{SqliteConnection, SqlitePool};

use crate::compress;
use crate::error::{AppError, Result, join_err};
use crate::models::Compression;

pub struct PreparedBlob {
    pub hash: Vec<u8>,
    pub mime: String,
    pub size_original: i64,
    /// None when the hash already existed (nothing to insert).
    pub payload: Option<(Vec<u8>, Compression)>,
    pub existed: bool,
}

pub async fn prepare_blob(pool: &SqlitePool, bytes: Vec<u8>, mime: &str) -> Result<PreparedBlob> {
    let mime = mime.to_string();
    let size_original = bytes.len() as i64;

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
        Some(
            tokio::task::spawn_blocking(move || compress::compress_for_storage(&m, bytes))
                .await
                .map_err(join_err)??,
        )
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
pub async fn insert_blob(conn: &mut SqliteConnection, b: &PreparedBlob) -> Result<i64> {
    if let Some((data, comp)) = &b.payload {
        sqlx::query(
            "INSERT INTO content (hash, mime_type, size_original, size_stored, compression_id, data)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(hash) DO NOTHING",
        )
        .bind(&b.hash)
        .bind(&b.mime)
        .bind(b.size_original)
        .bind(data.len() as i64)
        .bind(*comp)
        .bind(data)
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
