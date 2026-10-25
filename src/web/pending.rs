use crate::{
    db, dto,
    error::{AppError, OptionExt, Result},
    htmx::{HtmxResponse, PartialSwap},
    models,
    tag::clean_tags,
    template::FeedbackTemplate,
    time::ago_timestamp,
    url_canon::canonicalize_url,
};

use askama::Template;
use axum::{
    Form, Json,
    extract::{Path, Query, State},
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

impl<'a> From<&'a dto::PendingDto> for PendingFragmentTemplate<'a> {
    fn from(value: &'a dto::PendingDto) -> Self {
        PendingFragmentTemplate {
            id: value.id,
            url: &value.url,
            title: value.title.as_deref().unwrap_or(&value.url),
            status: value.status,
            created_at: ago_timestamp(value.created_at),
            tags: Vec::from_iter(value.tags.iter().map(|v| v.name.clone())).join(", "),
        }
    }
}

#[derive(Template)]
#[template(path = "pending.html")]
pub struct PendingTemplate<'a> {
    pub id: i64,
    pub url: &'a str,
    pub title: &'a str,
    pub status: models::PendingStatus,
    pub created_at: String,
    pub tags: String,
}

impl<'a> From<&'a dto::PendingDto> for PendingTemplate<'a> {
    fn from(value: &'a dto::PendingDto) -> Self {
        PendingTemplate {
            id: value.id,
            url: &value.url,
            title: value.title.as_deref().unwrap_or(&value.url),
            status: value.status,
            created_at: ago_timestamp(value.created_at),
            tags: Vec::from_iter(value.tags.iter().map(|v| v.name.clone())).join(", "),
        }
    }
}

pub async fn create_pending(
    State(pool): State<SqlitePool>,
    Form(req): Form<dto::CreatePending>,
) -> Result<HtmxResponse> {
    let canon = canonicalize_url(&req.url)?;
    let tags = clean_tags(&req.tags)?;

    let mut tx = pool.begin().await?;
    let id = match db::pending::insert_pending(&mut tx, &canon, req.title.as_deref(), &tags).await?
    {
        db::pending::Inserted::New(id) => id,
        db::pending::Inserted::Existing(existing_id) => {
            return Err(AppError::DuplicatePending { existing_id });
            // PartialSwap::tpl("#pending-feedback", &FeedbackTemplate::success("Removed"))?
            //     .swap("innerHTML"),
        }
    };
    tx.commit().await?;

    let pending_dto = db::pending::load_pending(&pool, id)
        .await?
        .or_not_found("pending item")?;

    Ok(HtmxResponse::partials([
        // New card goes to the top of the list
        PartialSwap::tpl(
            "#pending-list",
            &PendingFragmentTemplate::from(&pending_dto),
        )?
        .swap("afterbegin"),
        // Clear any previous error message
        PartialSwap::new("#pending-feedback", "").swap("innerHTML"),
    ]))
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

pub async fn delete_pending(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
) -> Result<HtmxResponse> {
    // Idempotent: if the row is already gone, the card should disappear anyway.
    db::pending::delete_pending(&pool, id).await?;

    // 200 with an empty body, not 204. htmx 4 doesn't swap 204 responses,
    // so the card would never be removed.
    Ok(HtmxResponse::partials([
        PartialSwap::new(format!("#pending-{id}"), "").swap("outerHTML"),
        PartialSwap::tpl("#pending-feedback", &FeedbackTemplate::success("Removed"))?
            .swap("innerHTML"),
    ]))
}
