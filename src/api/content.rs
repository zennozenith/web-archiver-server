use axum::{
    extract::State,
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_LENGTH, CONTENT_TYPE, ETAG},
    },
    response::{IntoResponse, Response},
};
use sqlx::SqlitePool;

use crate::{
    blob_store::BlobStore,
    db::contents::ContentRow,
    error::{AppError, OptionExt, Result},
    extract::ApiPath,
    serve::{etag_for, serve_content},
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
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair)
                .map_err(|_| AppError::BadRequest("invalid hash".into()))?;
            u8::from_str_radix(pair, 16).map_err(|_| AppError::BadRequest("invalid hash".into()))
        })
        .collect()
}

// GET /api/content/{hash}
pub(super) async fn get_content(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath(hash): ApiPath<String>,
) -> Result<Response> {
    let hash = parse_hash(&hash)?;
    let row = ContentRow::get_from_hash(&pool, hash)
        .await?
        .or_not_found("content")?;

    let ct = row
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());
    serve_content(&store, &headers, row, &ct, IMMUTABLE).await
}

// HEAD /api/content/{hash}   -> 200 if known, 404 otherwise (never loads the blob)
pub(super) async fn head_content(
    State(pool): State<SqlitePool>,
    ApiPath(hash): ApiPath<String>,
) -> Result<Response> {
    let hash = parse_hash(&hash)?;
    let (size, mime) = ContentRow::head(&pool, &hash)
        .await?
        .ok_or(AppError::NotFound("content"))?;

    let mut h = HeaderMap::new();
    h.insert(
        ETAG,
        HeaderValue::from_str(&etag_for(&hash)).map_err(|e| AppError::Internal(Box::new(e)))?,
    );
    h.insert(
        CONTENT_LENGTH,
        HeaderValue::from(u64::try_from(size).unwrap_or_default()),
    );
    h.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    if let Some(m) = mime.and_then(|m| HeaderValue::from_str(&m).ok()) {
        h.insert(CONTENT_TYPE, m);
    }
    Ok((StatusCode::OK, h).into_response())
}
