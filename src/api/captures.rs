use axum::{
    extract::State,
    http::HeaderMap,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde::Serialize;
use serde_json::Value;
use sqlx::{AssertSqlSafe, FromRow, SqlitePool};

use crate::{
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath},
    models::ContentRow,
    serve::{COLS, serve_content},
};

const CAPTURE_CACHE: &str = "private, max-age=86400";

#[derive(FromRow)]
struct CaptureDetailRow {
    id: i64,
    url_id: i64,
    url: String,
    version: i64,
    captured_at: i64,
    title: Option<String>,
    status_code: Option<i64>,
    content_type: Option<String>,
    original_encoding: Option<String>,
    request_headers: Option<String>,
    response_headers: Option<String>,
    metadata: Option<String>,
    error: Option<String>,
    redirect_url: Option<String>,
    html_hash: Option<String>,
    html_size: Option<i64>,
    html_stored: Option<i64>,
    screenshot_hash: Option<String>,
    screenshot_size: Option<i64>,
    screenshot_mime: Option<String>,
}

#[derive(Serialize)]
pub struct Blob {
    hash: String,
    size_original: i64,
    size_stored: Option<i64>,
    mime_type: Option<String>,
}

#[derive(Serialize)]
pub struct CaptureDetail {
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
    screenshot: Option<Blob>,
}

fn json(s: Option<String>) -> Option<Value> {
    s.and_then(|s| serde_json::from_str(&s).ok())
}

// GET /api/captures/{id}
pub async fn get_capture(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<CaptureDetail>> {
    let r: CaptureDetailRow = sqlx::query_as(
        "SELECT c.id, c.url_id, u.url,
                (SELECT COUNT(*) FROM capture c2
                  WHERE c2.url_id = c.url_id
                    AND (c2.captured_at, c2.id) <= (c.captured_at, c.id)) AS version,
                c.captured_at, c.title, c.status_code, c.content_type, c.original_encoding,
                c.request_headers, c.response_headers, c.metadata, c.error,
                ru.url AS redirect_url,
                NULLIF(lower(hex(h.hash)), '') AS html_hash,
                h.size_original AS html_size, h.size_stored AS html_stored,
                NULLIF(lower(hex(s.hash)), '') AS screenshot_hash,
                s.size_original AS screenshot_size, s.mime_type AS screenshot_mime
         FROM capture c
         JOIN url u ON u.id = c.url_id
         LEFT JOIN url ru ON ru.id = c.redirect_url_id
         LEFT JOIN content h ON h.id = c.content_id
         LEFT JOIN content s ON s.id = c.screenshot_content_id
         WHERE c.id = ?",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await?
    .or_not_found("capture")?;

    let html = r.html_hash.map(|hash| Blob {
        hash,
        size_original: r.html_size.unwrap_or(0),
        size_stored: r.html_stored,
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
        screenshot,
    }))
}

// DELETE /api/captures/{id}   (content rows are collected by the GC endpoint)
pub async fn delete_capture(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<StatusCode> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM capture_resource WHERE page_capture_id = ? OR resource_capture_id = ?",
    )
    .bind(id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let n = sqlx::query("DELETE FROM capture WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound("capture"));
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn serve_capture_blob(
    pool: &SqlitePool,
    headers: &HeaderMap,
    id: i64,
    join_col: &str, // fixed internal literal, never user input
    what: &'static str,
    force_type: Option<&str>,
) -> Result<Response> {
    let row: ContentRow = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT {COLS} FROM capture c JOIN content ct ON ct.id = c.{join_col} WHERE c.id = ?"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?
    .or_not_found(what)?;

    let ct = force_type
        .map(str::to_string)
        .or_else(|| row.mime_type.clone())
        .unwrap_or_else(|| "application/octet-stream".into());
    serve_content(headers, row, &ct, CAPTURE_CACHE).await
}

// GET /api/captures/{id}/html
pub async fn capture_html(
    State(pool): State<SqlitePool>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<i64>,
) -> Result<Response> {
    serve_capture_blob(
        &pool,
        &headers,
        id,
        "content_id",
        "archived html",
        Some("text/html; charset=utf-8"),
    )
    .await
}

// GET /api/captures/{id}/screenshot
pub async fn capture_screenshot(
    State(pool): State<SqlitePool>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<i64>,
) -> Result<Response> {
    serve_capture_blob(
        &pool,
        &headers,
        id,
        "screenshot_content_id",
        "screenshot",
        None,
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
    let id: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT id FROM capture WHERE url_id = ? AND {col} IS NOT NULL
         ORDER BY captured_at DESC, id DESC LIMIT 1"
    )))
    .bind(page_id)
    .fetch_optional(pool)
    .await?
    .or_not_found(what)?;
    // 307 (axum has no 302 helper); fine for GET
    Ok(Redirect::temporary(&format!("/api/captures/{id}/{suffix}")).into_response())
}

// GET /api/pages/{id}/latest/html
pub async fn latest_html(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Response> {
    latest_redirect(&pool, id, "content_id", "html", "archived html").await
}

// GET /api/pages/{id}/latest/screenshot
pub async fn latest_screenshot(
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
