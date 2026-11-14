//! Health, global installation statistics and maintenance.

use axum::{
    Router,
    extract::State,
    routing::{get, post},
};
use serde_json::json;

use crate::{
    app::AppState, db::stats, error::Result, identity::AuthUser, service,
    transport::extract::body::AppJson,
};

/// GET /api/health (unauthenticated)
pub(super) async fn health(State(state): State<AppState>) -> Result<AppJson<serde_json::Value>> {
    service::misc::health(&state).await?;
    Ok(AppJson(json!({ "status": "ok" })))
}

async fn stats(State(state): State<AppState>, user: AuthUser) -> Result<AppJson<stats::Stats>> {
    Ok(AppJson(service::misc::stats(&state, &user).await?))
}

async fn gc_content(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<AppJson<stats::GCStat>> {
    Ok(AppJson(service::misc::gc(&state, &user).await?))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/stats", get(stats))
        .route("/maintenance/gc", post(gc_content))
}
