//! Sub-resources saved alongside a capture (stylesheets, scripts, images, ...).

use axum::{extract::State, http::HeaderMap, response::Response};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::{
    blob_store::BlobStore,
    db::{captures::AssetRow, contents::ContentRow},
    error::{OptionExt, Result},
    extract::{ApiJson, ApiPath},
    serve::serve_content,
};

const ASSET_CACHE: &str = "private, max-age=86400";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AssetDto {
    content_id: i64,
    url: String,
    kind: String,
    status_code: Option<i64>,
    /// hex blake3; serve with GET /api/content/{hash} or the route below
    hash: String,
    mime_type: Option<String>,
    size_original: i64,
    size_stored: i64,
}

impl From<AssetRow> for AssetDto {
    fn from(row: AssetRow) -> Self {
        Self {
            content_id: row.content_id,
            url: row.url,
            kind: row.kind,
            status_code: row.status_code,
            hash: row.hash,
            mime_type: row.mime_type,
            size_original: row.size_original,
            size_stored: row.size_stored,
        }
    }
}

// GET /api/captures/{id}/assets
pub(super) async fn capture_assets(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<Vec<AssetDto>>> {
    let list = AssetRow::capture_assets(&pool, id)
        .await?
        .or_not_found("capture")?;
    Ok(ApiJson(list.into_iter().map(AssetDto::from).collect()))
}

// GET /api/pages/{id}/latest/assets
pub(super) async fn latest_assets(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<Vec<AssetDto>>> {
    let list = AssetRow::latest_assets(&pool, id)
        .await?
        .or_not_found("page")?;

    Ok(ApiJson(list.into_iter().map(AssetDto::from).collect()))
}

// GET /api/captures/{id}/assets/{content_id}
pub(super) async fn capture_asset(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath((capture_id, content_id)): ApiPath<(i64, i64)>,
) -> Result<Response> {
    let row: ContentRow = AssetRow::capture_asset(&pool, capture_id, content_id)
        .await?
        .or_not_found("asset")?;

    let content_type = row
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());
    serve_content(&store, &headers, row, &content_type, ASSET_CACHE).await
}
