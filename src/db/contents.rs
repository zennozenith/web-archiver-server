use sqlx::{AssertSqlSafe, Result, SqlitePool, prelude::FromRow};

use crate::{compress::Compression, db::captures::COLS, hash::Blake3Hash};

#[expect(
    dead_code,
    reason = "database row models mirror the schema; not every column is read yet"
)]
#[derive(Debug, FromRow)]
pub(crate) struct ContentRow {
    pub id: i64,
    pub hash: Vec<u8>, // 32 bytes
    pub mime_type: Option<String>,
    pub size_original: i64,
    pub size_stored: i64,
    pub compression_id: Compression,
    pub data: Option<Vec<u8>>,
    pub storage_ref: Option<String>,
    pub ref_count: i64,
    pub created_at: i64,
}

impl ContentRow {
    pub(crate) async fn get_from_hash(pool: &SqlitePool, hash: Blake3Hash) -> Result<Option<Self>> {
        let row = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT {COLS} FROM content ct WHERE ct.hash = ?"
        )))
        .bind(hash.as_slice())
        .fetch_optional(pool)
        .await?;

        Ok(row)
    }

    pub(crate) async fn get_from_id(
        pool: &SqlitePool,
        content_id: i64,
        join_col: &str, // fixed internal literal, never user input
    ) -> Result<Option<Self>> {
        let row = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT {COLS} FROM capture c JOIN content ct ON ct.id = c.{join_col} WHERE c.id = ?"
        )))
        .bind(content_id)
        .fetch_optional(pool)
        .await?;

        Ok(row)
    }

    pub(crate) async fn head(
        pool: &SqlitePool,
        hash: &Blake3Hash,
    ) -> Result<Option<(i64, Option<String>)>> {
        let found: Option<(i64, Option<String>)> =
            sqlx::query_as("SELECT size_original, mime_type FROM content WHERE hash = ?")
                .bind(hash.as_slice())
                .fetch_optional(pool)
                .await?;
        Ok(found)
    }
}
