//! Sub-resources saved alongside a capture (stylesheets, scripts, images, ...).

use axum::{extract::State, http::HeaderMap, response::Response};
use serde::Serialize;
use sqlx::{AssertSqlSafe, FromRow, SqlitePool};

use crate::{
    blob_store::BlobStore,
    error::{OptionExt, Result},
    extract::{ApiJson, ApiPath},
    models::ContentRow,
    serve::{COLS, serve_content},
};

const ASSET_CACHE: &str = "private, max-age=86400";

#[derive(FromRow)]
struct AssetRow {
    content_id: i64,
    url: String,
    kind: String,
    status_code: Option<i64>,
    hash: String,
    mime_type: Option<String>,
    size_original: i64,
    size_stored: i64,
}

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

async fn list_assets(pool: &SqlitePool, capture_id: i64) -> Result<Vec<AssetDto>> {
    let rows: Vec<AssetRow> = sqlx::query_as(
        "SELECT ca.content_id, ca.url, ca.kind, ca.status_code,
                lower(hex(ct.hash)) AS hash, ct.mime_type,
                ct.size_original, ct.size_stored
         FROM capture_asset ca
         JOIN content ct ON ct.id = ca.content_id
         WHERE ca.capture_id = ?
         ORDER BY ca.kind, ca.url",
    )
    .bind(capture_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(AssetDto::from).collect())
}

// GET /api/captures/{id}/assets
pub(super) async fn capture_assets(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<Vec<AssetDto>>> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM capture WHERE id = ?")
        .bind(id)
        .fetch_optional(&pool)
        .await?
        .or_not_found("capture")?;
    Ok(ApiJson(list_assets(&pool, id).await?))
}

// GET /api/pages/{id}/latest/assets
pub(super) async fn latest_assets(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<Vec<AssetDto>>> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM url WHERE id = ?")
        .bind(id)
        .fetch_optional(&pool)
        .await?
        .or_not_found("page")?;

    let capture_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM capture WHERE url_id = ?
         ORDER BY captured_at DESC, id DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await?;

    match capture_id {
        Some(capture_id) => Ok(ApiJson(list_assets(&pool, capture_id).await?)),
        None => Ok(ApiJson(Vec::new())),
    }
}

// GET /api/captures/{id}/assets/{content_id}
pub(super) async fn capture_asset(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    headers: HeaderMap,
    ApiPath((capture_id, content_id)): ApiPath<(i64, i64)>,
) -> Result<Response> {
    let row: ContentRow = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT {COLS}
         FROM capture_asset ca
         JOIN content ct ON ct.id = ca.content_id
         WHERE ca.capture_id = ? AND ca.content_id = ?
         LIMIT 1"
    )))
    .bind(capture_id)
    .bind(content_id)
    .fetch_optional(&pool)
    .await?
    .or_not_found("asset")?;

    let content_type = row
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());
    serve_content(&store, &headers, row, &content_type, ASSET_CACHE).await
}
