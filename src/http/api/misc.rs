//! Health, global installation statistics and maintenance.

use axum::{
    Router,
    extract::State,
    routing::{get, post},
};
use serde_json::json;

use crate::{
    app::AppState, auth::AuthUser, db::stats, error::Result, http::extract::body::AppJson, service,
};

/// GET /api/health (unauthenticated)
pub(super) async fn health(State(ctx): State<AppState>) -> Result<AppJson<serde_json::Value>> {
    service::misc::health(&ctx).await?;
    Ok(AppJson(json!({ "status": "ok" })))
}

async fn stats(State(ctx): State<AppState>, user: AuthUser) -> Result<AppJson<stats::Stats>> {
    Ok(AppJson(service::misc::stats(&ctx, &user).await?))
}

async fn gc_content(State(ctx): State<AppState>, user: AuthUser) -> Result<AppJson<stats::GCStat>> {
    Ok(AppJson(service::misc::gc(&ctx, &user).await?))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/stats", get(stats))
        .route("/maintenance/gc", post(gc_content))
}
