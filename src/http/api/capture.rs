use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::{
    AppState,
    blob_store::BlobStore,
    db::{
        capture::{self, CaptureBlob, CaptureDetail, CaptureId},
        content::Content,
        page::PageId,
    },
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath},
    hash::Blake3Hash,
    mime::MimeType,
    serve::{CSP, CSP_ARCHIVED_HTML, serve_content_with_csp},
};

const CAPTURE_CACHE_CONTROL: &str = "private, max-age=86400";

// ── Blob routes ─────────────────────────────────────────────────────────────

/// How one kind of capture blob is exposed over HTTP.
struct BlobRoute {
    blob: CaptureBlob,
    /// Human-readable name used in 404 messages.
    label: &'static str,
    /// Path segment under `/api/capture/{id}/`.
    url_suffix: &'static str,
    /// Overrides the stored MIME type when set.
    content_type: Option<&'static str>,
    csp: &'static str,
}

const HTML: BlobRoute = BlobRoute {
    blob: CaptureBlob::Html,
    label: "archived html",
    url_suffix: "html",
    content_type: Some("text/html; charset=utf-8"),
    csp: CSP_ARCHIVED_HTML,
};

const ORIGINAL_HTML: BlobRoute = BlobRoute {
    blob: CaptureBlob::OriginalHtml,
    label: "original html",
    url_suffix: "html/original",
    content_type: Some("text/html; charset=utf-8"),
    csp: CSP_ARCHIVED_HTML,
};

const SCREENSHOT: BlobRoute = BlobRoute {
    blob: CaptureBlob::Screenshot,
    label: "screenshot",
    url_suffix: "screenshot",
    content_type: None,
    csp: CSP,
};

async fn serve_thumbnail(
    pool: &SqlitePool,
    store: &BlobStore,
    headers: &HeaderMap,
    page_id: PageId,
) -> Result<Response> {
    let content = Content::for_page(pool, page_id)
        .await?
        .or_not_found("thumbnail not found")?;

    let content_type = content
        .mime_type
        .as_str()
        .to_owned()
        .parse()
        .unwrap_or_default();

    serve_content_with_csp(
        store,
        headers,
        content,
        &content_type,
        CAPTURE_CACHE_CONTROL,
        CSP,
    )
    .await
}

async fn serve_blob(
    pool: &SqlitePool,
    store: &BlobStore,
    headers: &HeaderMap,
    capture_id: CaptureId,
    route: &BlobRoute,
) -> Result<Response> {
    let content = Content::for_capture(pool, capture_id, route.blob)
        .await?
        .or_not_found(route.label)?;

    let content_type = route
        .content_type
        .unwrap_or_else(|| content.mime_type.as_str())
        .to_owned()
        .parse()
        .unwrap_or_default();

    serve_content_with_csp(
        store,
        headers,
        content,
        &content_type,
        CAPTURE_CACHE_CONTROL,
        route.csp,
    )
    .await
}

/// Redirects to the page's newest capture that has this blob.
async fn redirect_to_latest(
    pool: &SqlitePool,
    page_id: PageId,
    route: &BlobRoute,
) -> Result<Response> {
    let capture_id = route
        .blob
        .latest_capture_id(pool, page_id)
        .await?
        .or_not_found(route.label)?;

    // 307, fine for GET.
    let target = format!("/api/capture/{capture_id}/{}", route.url_suffix);
    Ok(Redirect::temporary(&target).into_response())
}

// ── Capture detail ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BlobSummary {
    hash: Blake3Hash,
    size_original: i64,
    size_stored: i64,
    mime_type: MimeType,
}

impl BlobSummary {
    /// `None` when the capture has no such blob (no hash).
    fn new(
        hash: Option<Blake3Hash>,
        size_original: Option<i64>,
        size_stored: Option<i64>,
        mime_type: MimeType,
    ) -> Option<Self> {
        if let (Some(hash), Some(size_original), Some(size_stored)) =
            (hash, size_original, size_stored)
        {
            Some(Self {
                hash,
                size_original,
                size_stored,
                mime_type,
            })
        } else {
            None
        }
    }

    fn html(hash: Option<Blake3Hash>, size: Option<i64>, stored: Option<i64>) -> Option<Self> {
        Self::new(
            hash,
            size,
            stored,
            "text/html".parse::<MimeType>().unwrap_or_default(),
        )
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CaptureDetailResponse {
    id: CaptureId,
    page_id: PageId,
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
    html: Option<BlobSummary>,
    original_html: Option<BlobSummary>,
    screenshot: Option<BlobSummary>,
    asset_count: i64,
}

/// Parses a JSON text column; malformed values are reported as absent.
fn parse_json_column(raw: Option<String>) -> Option<Value> {
    raw.and_then(|text| serde_json::from_str(&text).ok())
}

impl From<CaptureDetail> for CaptureDetailResponse {
    fn from(d: CaptureDetail) -> Self {
        Self {
            id: d.id,
            page_id: d.page_id,
            url: d.url,
            version: d.version,
            captured_at: d.captured_at,
            title: d.title,
            status_code: d.status_code,
            content_type: d.content_type,
            original_encoding: d.original_encoding,
            redirect_url: d.redirect_url,
            error: d.error,
            request_headers: parse_json_column(d.request_headers),
            response_headers: parse_json_column(d.response_headers),
            metadata: parse_json_column(d.metadata),
            html: BlobSummary::html(d.html_hash, d.html_size, d.html_stored),
            original_html: BlobSummary::html(
                d.original_html_hash,
                d.original_html_size,
                d.original_html_stored,
            ),
            screenshot: BlobSummary::new(
                d.screenshot_hash,
                d.screenshot_size,
                d.screenshot_size,
                d.screenshot_mime,
            ),
            asset_count: d.asset_count,
        }
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

// GET /page/{id}/thumbnail
pub(super) async fn get_thumbnail(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<PageId>,
) -> Result<Response> {
    serve_thumbnail(&pool, &store, &headers, id).await
}

// GET /api/capture/{id}
pub(super) async fn get_capture(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<CaptureId>,
) -> Result<ApiJson<CaptureDetailResponse>> {
    let detail = CaptureDetail::load(&pool, id)
        .await?
        .or_not_found("capture")?;
    Ok(ApiJson(detail.into()))
}

// DELETE /api/capture/{id}   (content rows are collected by the GC endpoint)
pub(super) async fn delete_capture(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<CaptureId>,
) -> Result<StatusCode> {
    if !capture::delete(&pool, id).await? {
        return Err(AppError::NotFound("capture"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// GET /api/capture/{id}/html   (rewritten: resource URLs point at /api/content)
pub(super) async fn get_capture_html(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<CaptureId>,
) -> Result<Response> {
    serve_blob(&pool, &store, &headers, id, &HTML).await
}

// GET /api/capture/{id}/html/original   (exact bytes uploaded to /api/archive)
pub(super) async fn get_capture_original_html(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<CaptureId>,
) -> Result<Response> {
    serve_blob(&pool, &store, &headers, id, &ORIGINAL_HTML).await
}

// GET /api/capture/{id}/screenshot
pub(super) async fn get_capture_screenshot(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<CaptureId>,
) -> Result<Response> {
    serve_blob(&pool, &store, &headers, id, &SCREENSHOT).await
}

// GET /api/page/{id}/latest/html
pub(super) async fn redirect_latest_html(
    State(pool): State<SqlitePool>,
    ApiPath(page_id): ApiPath<PageId>,
) -> Result<Response> {
    redirect_to_latest(&pool, page_id, &HTML).await
}

// GET /api/page/{id}/latest/html/original
pub(super) async fn redirect_latest_original_html(
    State(pool): State<SqlitePool>,
    ApiPath(page_id): ApiPath<PageId>,
) -> Result<Response> {
    redirect_to_latest(&pool, page_id, &ORIGINAL_HTML).await
}

// GET /api/page/{id}/latest/screenshot
pub(super) async fn redirect_latest_screenshot(
    State(pool): State<SqlitePool>,
    ApiPath(page_id): ApiPath<PageId>,
) -> Result<Response> {
    redirect_to_latest(&pool, page_id, &SCREENSHOT).await
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/capture/{id}", get(get_capture).delete(delete_capture))
        .route("/capture/{id}/html", get(get_capture_html))
        .route(
            "/capture/{id}/html/original",
            get(get_capture_original_html),
        )
        .route("/capture/{id}/screenshot", get(get_capture_screenshot))
        .route("/capture/{id}/asset", get(super::asset::capture_assets))
        .route(
            "/capture/{id}/asset/{content_id}",
            get(super::asset::capture_asset),
        )
        .route("/page/{id}/latest/asset", get(super::asset::latest_assets))
}
