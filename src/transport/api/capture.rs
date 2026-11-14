//! Capture metadata and blob routes.

use axum::{
    Router,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
    routing::get,
};

use crate::{
    app::AppState,
    db::{
        capture::{CaptureBlob, CaptureId},
        page::PageId,
    },
    error::Result,
    identity::AuthUser,
    service,
    transport::{
        cache,
        dto::archive::CaptureDetailResponse,
        extract::body::{AppJson, AppPath},
        serve::{CSP, CSP_ARCHIVED_HTML, serve_content_with_csp},
    },
    util::mime::MimeType,
};

/// How one kind of capture blob is exposed over HTTP.
struct BlobRoute {
    blob: CaptureBlob,
    url_suffix: &'static str,
    content_type: Option<&'static str>,
    csp: &'static str,
}

const HTML: BlobRoute = BlobRoute {
    blob: CaptureBlob::Html,
    url_suffix: "html",
    content_type: Some("text/html; charset=utf-8"),
    csp: CSP_ARCHIVED_HTML,
};

const ORIGINAL_HTML: BlobRoute = BlobRoute {
    blob: CaptureBlob::OriginalHtml,
    url_suffix: "html/original",
    content_type: Some("text/html; charset=utf-8"),
    csp: CSP_ARCHIVED_HTML,
};

const SCREENSHOT: BlobRoute = BlobRoute {
    blob: CaptureBlob::Screenshot,
    url_suffix: "screenshot",
    content_type: None,
    csp: CSP,
};

fn mime_for(override_type: Option<&str>, fallback: &MimeType) -> MimeType {
    override_type
        .unwrap_or_else(|| fallback.as_str())
        .parse()
        .unwrap_or_default()
}

async fn serve_blob(
    state: &AppState,
    actor: &AuthUser,
    headers: &HeaderMap,
    capture_id: CaptureId,
    route: &BlobRoute,
) -> Result<Response> {
    let content = service::capture::blob(&state.db, actor, capture_id, route.blob).await?;
    let content_type = mime_for(route.content_type, &content.mime_type);

    serve_content_with_csp(
        &state.store,
        headers,
        content,
        &content_type,
        cache::PRIVATE_DAY,
        route.csp,
    )
    .await
}

async fn redirect_to_latest(
    state: &AppState,
    actor: &AuthUser,
    page_id: PageId,
    route: &BlobRoute,
) -> Result<Response> {
    let capture_id =
        service::capture::latest_with_blob(&state.db, actor, page_id, route.blob).await?;
    let target = format!("/api/capture/{capture_id}/{}", route.url_suffix);
    Ok(Redirect::temporary(&target).into_response())
}

/// GET /page/{id}/thumbnail
pub(super) async fn get_thumbnail(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    AppPath(id): AppPath<PageId>,
) -> Result<Response> {
    let content = service::capture::page_thumbnail(&state.db, &user, id).await?;
    let content_type = content.mime_type.clone();
    serve_content_with_csp(
        &state.store,
        &headers,
        content,
        &content_type,
        cache::PRIVATE_DAY,
        CSP,
    )
    .await
}

async fn get_capture(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CaptureId>,
) -> Result<AppJson<CaptureDetailResponse>> {
    let detail = service::capture::detail(&state.db, &user, id).await?;
    Ok(AppJson(detail.into()))
}

async fn delete_capture(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CaptureId>,
) -> Result<axum::http::StatusCode> {
    service::capture::delete(&state.db, &user, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn get_capture_html(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    AppPath(id): AppPath<CaptureId>,
) -> Result<Response> {
    serve_blob(&state, &user, &headers, id, &HTML).await
}

async fn get_capture_original_html(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    AppPath(id): AppPath<CaptureId>,
) -> Result<Response> {
    serve_blob(&state, &user, &headers, id, &ORIGINAL_HTML).await
}

async fn get_capture_screenshot(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    AppPath(id): AppPath<CaptureId>,
) -> Result<Response> {
    serve_blob(&state, &user, &headers, id, &SCREENSHOT).await
}

pub(super) async fn redirect_latest_html(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(page_id): AppPath<PageId>,
) -> Result<Response> {
    redirect_to_latest(&state, &user, page_id, &HTML).await
}

pub(super) async fn redirect_latest_original_html(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(page_id): AppPath<PageId>,
) -> Result<Response> {
    redirect_to_latest(&state, &user, page_id, &ORIGINAL_HTML).await
}

pub(super) async fn redirect_latest_screenshot(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(page_id): AppPath<PageId>,
) -> Result<Response> {
    redirect_to_latest(&state, &user, page_id, &SCREENSHOT).await
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
