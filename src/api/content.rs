use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header::*},
    response::{IntoResponse, Response},
};
use sqlx::{AssertSqlSafe, SqlitePool};

use crate::{
    blob_store::BlobStore,
    error::{AppError, OptionExt, Result},
    extract::ApiPath,
    models::ContentRow,
    serve::{COLS, etag_for, serve_content},
};

// Content is immutable by hash. "private" because the API sits behind bearer auth;
// switch to "public" if you serve it unauthenticated behind a CDN.
const IMMUTABLE: &str = "private, max-age=31536000, immutable";

fn parse_hash(s: &str) -> Result<Vec<u8>> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest(
            "hash must be 64 hex characters".into(),
        ));
    }
    (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<std::result::Result<Vec<u8>, _>>()
        .map_err(|_| AppError::BadRequest("invalid hash".into()))
}

// GET /api/content/{hash}
pub async fn get_content(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath(hash): ApiPath<String>,
) -> Result<Response> {
    let hash = parse_hash(&hash)?;
    let row: ContentRow = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT {COLS} FROM content ct WHERE ct.hash = ?"
    )))
    .bind(&hash)
    .fetch_optional(&pool)
    .await?
    .or_not_found("content")?;

    let ct = row
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());
    serve_content(&store, &headers, row, &ct, IMMUTABLE).await
}

// HEAD /api/content/{hash}   -> 200 if known, 404 otherwise (never loads the blob)
pub async fn head_content(
    State(pool): State<SqlitePool>,
    ApiPath(hash): ApiPath<String>,
) -> Result<Response> {
    let hash = parse_hash(&hash)?;
    let found: Option<(i64, Option<String>)> =
        sqlx::query_as("SELECT size_original, mime_type FROM content WHERE hash = ?")
            .bind(&hash)
            .fetch_optional(&pool)
            .await?;
    let (size, mime) = found.ok_or(AppError::NotFound("content"))?;

    let mut h = HeaderMap::new();
    h.insert(
        ETAG,
        HeaderValue::from_str(&etag_for(&hash)).map_err(|e| AppError::Internal(Box::new(e)))?,
    );
    h.insert(CONTENT_LENGTH, HeaderValue::from(size as u64));
    h.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    if let Some(m) = mime.and_then(|m| HeaderValue::from_str(&m).ok()) {
        h.insert(CONTENT_TYPE, m);
    }
    Ok((StatusCode::OK, h).into_response())
}
