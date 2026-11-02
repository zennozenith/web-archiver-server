//! Sub-resources saved alongside a capture (stylesheets, scripts, images, ...).

use axum::{extract::State, http::HeaderMap, response::Response};
use sqlx::SqlitePool;

use crate::{
    blob_store::BlobStore,
    db::{capture::CaptureAsset, content::Content},
    error::{OptionExt, Result},
    extract::{ApiJson, ApiPath},
    serve::serve_content,
};

const ASSET_CACHE: &str = "private, max-age=86400";

// GET /api/captures/{id}/assets
pub(super) async fn capture_assets(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<Vec<CaptureAsset>>> {
    let list = CaptureAsset::list(&pool, id.into()).await?;
    Ok(ApiJson(list.into_iter().collect()))
}

// GET /api/pages/{id}/latest/assets
pub(super) async fn latest_assets(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<Vec<CaptureAsset>>> {
    let list = CaptureAsset::for_latest_capture(&pool, id.into())
        .await?
        .or_not_found("page")?;

    Ok(ApiJson(list.into_iter().collect()))
}

// GET /api/captures/{id}/assets/{content_id}
pub(super) async fn capture_asset(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath((capture_id, content_id)): ApiPath<(i64, i64)>,
) -> Result<Response> {
    let row = Content::for_capture_asset(&pool, capture_id.into(), content_id.into())
        .await?
        .or_not_found("asset")?;

    let content_type = row.mime_type.clone();
    serve_content(&store, &headers, row, &content_type, ASSET_CACHE).await
}
