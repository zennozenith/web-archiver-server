use axum::{
    Router,
    extract::State,
    routing::{get, post},
};
use serde_json::json;
use sqlx::SqlitePool;

use crate::{db::stats, http::extract::body::AppJson, infra::BlobStore, prelude::*};

// GET /api/health  (unauthenticated)
#[axum::debug_handler]
pub(super) async fn health(State(pool): State<SqlitePool>) -> Result<AppJson<serde_json::Value>> {
    stats::health(&pool).await?;
    Ok(AppJson(json!({ "status": "ok" })))
}

// GET /api/stats
async fn stats(State(pool): State<SqlitePool>) -> Result<AppJson<stats::Stats>> {
    let stat = stats::stats(&pool).await?;
    Ok(AppJson(stat))
}

// POST /api/maintenance/gc
async fn gc_content(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
) -> Result<AppJson<stats::GCStat>> {
    let gc_stat = stats::gc_content(&pool, &store).await?;
    Ok(AppJson(gc_stat))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/stats", get(stats))
        .route("/maintenance/gc", post(gc_content))
}
