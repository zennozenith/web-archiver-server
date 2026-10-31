use axum::extract::State;
use sqlx::SqlitePool;

use crate::{blob_store::BlobStore, db::stats, error::Result, extract::ApiJson};

// GET /api/health  (unauthenticated)
pub(super) async fn health(State(pool): State<SqlitePool>) -> Result<ApiJson<serde_json::Value>> {
    stats::health(&pool).await?;
    Ok(ApiJson(serde_json::json!({ "status": "ok" })))
}

// GET /api/stats
pub(super) async fn stats(State(pool): State<SqlitePool>) -> Result<ApiJson<stats::Stats>> {
    let stat = stats::stats(&pool).await?;
    Ok(ApiJson(stat))
}

// POST /api/maintenance/gc
pub(super) async fn gc_content(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
) -> Result<ApiJson<stats::GCStat>> {
    let gc_stat = stats::gc_content(&pool, &store).await?;
    Ok(ApiJson(gc_stat))
}
