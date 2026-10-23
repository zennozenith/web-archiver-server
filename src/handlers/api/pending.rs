use axum::{
    Json, debug_handler,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    db,
    dto::{CreatePending, Page, PendingDto},
    error::Result,
    extract::{ApiJson, JsonOrForm},
    models::PendingStatus,
};

pub async fn create_pending(
    State(pool): State<SqlitePool>,
    JsonOrForm(req): JsonOrForm<CreatePending>,
) -> Result<(StatusCode, ApiJson<PendingDto>)> {
    Ok((
        StatusCode::CREATED,
        ApiJson(db::pending::create_pending(&pool, req).await?),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }
}

#[derive(Deserialize)]
pub struct QueryPending {
    limit: Option<u32>,
    status: Option<PendingStatus>,
    after_id: Option<i64>,
}

#[debug_handler]
pub async fn get_pending(
    State(pool): State<SqlitePool>,
    Query(query): Query<QueryPending>,
) -> Result<Json<Page<PendingDto>>> {
    let QueryPending {
        limit,
        status,
        after_id,
    } = query;
    // TODO: hardcoded limit
    let limit = limit.unwrap_or(5);
    Ok(Json(
        db::pending::list_pending(&pool, status, after_id, limit).await?,
    ))
}
