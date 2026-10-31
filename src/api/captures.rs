use axum::{
    extract::State,
    http::HeaderMap,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::{
    blob_store::BlobStore,
    db::{
        captures::{AssetRow, CaptureDetailRow},
        contents::ContentRow,
    },
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath},
    serve::{CSP, CSP_ARCHIVED_HTML, serve_content_with_csp},
};

const CAPTURE_CACHE: &str = "private, max-age=86400";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Blob {
    hash: String,
    size_original: i64,
    size_stored: Option<i64>,
    mime_type: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CaptureDetail {
    id: i64,
    page_id: i64,
    url: String,
    version: i64,
    captured_at: i64,
    title: Option<String>,
    status_code: Option<i64>,
    content_type: Option<String>,
    original_encoding: Option<String>,
    redirect_url: Option<String>,
    error: Option<String>,
    request_headers: Option<Value>,
    response_headers: Option<Value>,
    metadata: Option<Value>,
    html: Option<Blob>,
    original_html: Option<Blob>,
    screenshot: Option<Blob>,
    asset_count: i64,
}

fn json(s: Option<String>) -> Option<Value> {
    s.and_then(|s| serde_json::from_str(&s).ok())
}

// GET /api/captures/{id}
pub(super) async fn get_capture(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<CaptureDetail>> {
    let r = CaptureDetailRow::get(&pool, id)
        .await?
        .or_not_found("capture")?;

    let html = r.html_hash.map(|hash| Blob {
        hash,
        size_original: r.html_size.unwrap_or(0),
        size_stored: r.html_stored,
        mime_type: Some("text/html".into()),
    });
    let original_html = r.original_html_hash.map(|hash| Blob {
        hash,
        size_original: r.original_html_size.unwrap_or(0),
        size_stored: r.original_html_stored,
        mime_type: Some("text/html".into()),
    });
    let screenshot = r.screenshot_hash.map(|hash| Blob {
        hash,
        size_original: r.screenshot_size.unwrap_or(0),
        size_stored: r.screenshot_size,
        mime_type: r.screenshot_mime,
    });

    Ok(ApiJson(CaptureDetail {
        id: r.id,
        page_id: r.url_id,
        url: r.url,
        version: r.version,
        captured_at: r.captured_at,
        title: r.title,
        status_code: r.status_code,
        content_type: r.content_type,
        original_encoding: r.original_encoding,
        redirect_url: r.redirect_url,
        error: r.error,
        request_headers: json(r.request_headers),
        response_headers: json(r.response_headers),
        metadata: json(r.metadata),
        html,
        original_html,
        screenshot,
        asset_count: r.asset_count,
    }))
}

// DELETE /api/captures/{id}   (content rows are collected by the GC endpoint)
pub(super) async fn delete_capture(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<StatusCode> {
    let n = CaptureDetailRow::delete(&pool, id).await?;
    if n == 0 {
        return Err(AppError::NotFound("capture"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[expect(
    clippy::too_many_arguments,
    reason = "keeps each blob route's policy explicit at the call site"
)]
async fn serve_capture_blob(
    pool: &SqlitePool,
    store: &BlobStore,
    headers: &HeaderMap,
    id: i64,
    join_col: &str, // fixed internal literal, never user input
    what: &'static str,
    force_type: Option<&str>,
    csp: &'static str,
) -> Result<Response> {
    let row: ContentRow = ContentRow::get_from_id(pool, id, join_col)
        .await?
        .or_not_found(what)?;

    let ct = force_type
        .map(str::to_owned)
        .or_else(|| row.mime_type.clone())
        .unwrap_or_else(|| "application/octet-stream".into());
    serve_content_with_csp(store, headers, row, &ct, CAPTURE_CACHE, csp).await
}

// GET /api/captures/{id}/html   (rewritten: resource URLs point at /api/content)
pub(super) async fn capture_html(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<i64>,
) -> Result<Response> {
    serve_capture_blob(
        &pool,
        &store,
        &headers,
        id,
        "content_id",
        "archived html",
        Some("text/html; charset=utf-8"),
        CSP_ARCHIVED_HTML,
    )
    .await
}

// GET /api/captures/{id}/html/original   (exact bytes uploaded to /api/archive)
pub(super) async fn capture_original_html(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<i64>,
) -> Result<Response> {
    serve_capture_blob(
        &pool,
        &store,
        &headers,
        id,
        "original_content_id",
        "original html",
        Some("text/html; charset=utf-8"),
        CSP_ARCHIVED_HTML,
    )
    .await
}

// GET /api/captures/{id}/screenshot
pub(super) async fn capture_screenshot(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<i64>,
) -> Result<Response> {
    serve_capture_blob(
        &pool,
        &store,
        &headers,
        id,
        "screenshot_content_id",
        "screenshot",
        None,
        CSP,
    )
    .await
}

async fn latest_redirect(
    pool: &SqlitePool,
    page_id: i64,
    col: &str,
    suffix: &str,
    what: &'static str,
) -> Result<Response> {
    let id: i64 = AssetRow::where_not_null(pool, page_id, col)
        .await?
        .or_not_found(what)?;
    // 307 (axum has no 302 helper); fine for GET
    Ok(Redirect::temporary(&format!("/api/captures/{id}/{suffix}")).into_response())
}

// GET /api/pages/{id}/latest/html
pub(super) async fn latest_html(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Response> {
    latest_redirect(&pool, id, "content_id", "html", "archived html").await
}

// GET /api/pages/{id}/latest/html/original
pub(super) async fn latest_original_html(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Response> {
    latest_redirect(
        &pool,
        id,
        "original_content_id",
        "html/original",
        "original html",
    )
    .await
}

// GET /api/pages/{id}/latest/screenshot
pub(super) async fn latest_screenshot(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Response> {
    latest_redirect(
        &pool,
        id,
        "screenshot_content_id",
        "screenshot",
        "screenshot",
    )
    .await
}
