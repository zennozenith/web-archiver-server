use crate::{db, dto, error::Result, extract::ApiJson, models};

use askama::Template;
use axum::{
    Form, Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use sqlx::SqlitePool;

#[derive(Template)]
#[template(path = "fragments/pending-card.html")]
pub struct PendingFragmentTemplate<'a> {
    pub id: i64,
    pub url: &'a str,
    pub title: &'a str,
    pub status: models::PendingStatus,
    pub created_at: String,
    pub tags: String,
}

pub async fn create_pending(
    State(pool): State<SqlitePool>,
    Form(req): Form<dto::CreatePending>,
) -> Result<(StatusCode, ApiJson<dto::PendingDto>)> {
    Ok((
        StatusCode::CREATED,
        ApiJson(db::pending::create_pending(&pool, req).await?),
    ))
}

#[derive(Deserialize)]
pub struct QueryPending {
    limit: Option<u32>,
    status: Option<models::PendingStatus>,
    after_id: Option<i64>,
}

pub async fn get_pending(
    State(pool): State<SqlitePool>,
    Query(query): Query<QueryPending>,
) -> Result<Json<dto::Page<dto::PendingDto>>> {
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
