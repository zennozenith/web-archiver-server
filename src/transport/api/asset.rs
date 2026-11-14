//! Sub-resources saved alongside a capture (stylesheets, scripts, images, ...).

use axum::{extract::State, http::HeaderMap, response::Response};

use crate::{
    app::AppState,
    db::{capture::CaptureAsset, page::PageId},
    error::Result,
    identity::AuthUser,
    service,
    transport::{
        cache,
        extract::body::{AppJson, AppPath},
        serve::serve_content,
    },
};

pub(super) async fn capture_assets(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<i64>,
) -> Result<AppJson<Vec<CaptureAsset>>> {
    Ok(AppJson(
        service::capture::assets(&state.db, &user, id.into()).await?,
    ))
}

pub(super) async fn latest_assets(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<i64>,
) -> Result<AppJson<Vec<CaptureAsset>>> {
    Ok(AppJson(
        service::capture::latest_assets(&state.db, &user, PageId::from(id)).await?,
    ))
}

pub(super) async fn capture_asset(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    AppPath((capture_id, content_id)): AppPath<(i64, i64)>,
) -> Result<Response> {
    let row = service::capture::asset_blob(&state.db, &user, capture_id.into(), content_id).await?;
    let content_type = row.mime_type.clone();
    serve_content(
        &state.blobs,
        &headers,
        row,
        &content_type,
        cache::PRIVATE_1D,
    )
    .await
}
