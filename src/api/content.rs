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
    db::content::{Content, ContentHead},
    error::{AppError, OptionExt, Result},
    extract::ApiPath,
    hash::Blake3Hash,
    serve::{etag_for, serve_content},
};

const CONTENT: &str = "content";

/// Content is immutable by hash. "private" because the API sits behind bearer auth;
/// switch to "public" if you serve it unauthenticated behind a CDN.
const IMMUTABLE_CACHE_CONTROL: &str = "private, max-age=31536000, immutable";

// GET /api/content/{hash}
pub(super) async fn get_content(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath(hash): ApiPath<String>,
) -> Result<Response> {
    let hash: Blake3Hash = hash.parse()?;
    let content = Content::by_hash(&pool, &hash)
        .await?
        .or_not_found(CONTENT)?;

    let content_type = content.mime_type.clone();
    serve_content(
        &store,
        &headers,
        content,
        &content_type,
        IMMUTABLE_CACHE_CONTROL,
    )
    .await
}

// HEAD /api/content/{hash}   -> 200 if known, 404 otherwise (never loads the blob)
pub(super) async fn head_content(
    State(pool): State<SqlitePool>,
    ApiPath(hash): ApiPath<String>,
) -> Result<Response> {
    let hash: Blake3Hash = hash.parse()?;
    let head = ContentHead::by_hash(&pool, &hash)
        .await?
        .ok_or(AppError::NotFound(CONTENT))?;

    let etag =
        HeaderValue::from_str(&etag_for(&hash)).map_err(|e| AppError::Internal(Box::new(e)))?;
    let length = u64::try_from(head.size_original).unwrap_or_default();

    let mut headers = HeaderMap::new();
    headers.insert(ETAG, etag);
    headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    if let Ok(mime) = HeaderValue::from_str(head.mime_type.as_str()) {
        headers.insert(CONTENT_TYPE, mime);
    }
    Ok((StatusCode::OK, headers).into_response())
}
