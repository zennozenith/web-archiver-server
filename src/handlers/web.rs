use crate::{
    core::RenderHtml as _,
    db, dto,
    error::{AppError, Result},
    models,
    time::ago_timestamp,
};

use askama::Template;
use axum::{extract::State, response::IntoResponse};
use sqlx::SqlitePool;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    pendings: Vec<PendingFragmentTemplate<'a>>,
    health: dto::HealthDto,
}

#[derive(Template)]
#[template(path = "fragments/pending-card.html")]
struct PendingFragmentTemplate<'a> {
    pub id: i64,
    pub url: &'a str,
    pub title: &'a str,
    pub status: models::PendingStatus,
    pub created_at: String,
    pub tags: String,
}

#[derive(Template)]
#[template(path = "vite-counter.html")]
struct ViteCounter<'a> {
    title: &'a str,
}

pub async fn index(State(pool): State<SqlitePool>) -> Result<impl IntoResponse, AppError> {
    const LIMIT: u32 = 5;

    // OPTIMIZE: join both database calls
    let health = db::health::health(&pool).await?;

    let dto::Page { items, .. } = db::pending::list_pending(&pool, None, None, LIMIT).await?;
    let pendings = items
        .iter()
        .map(
            |dto::PendingDto {
                 id,
                 url,
                 title,
                 status,
                 created_at,
                 tags,
                 ..
             }| {
                PendingFragmentTemplate {
                    id: *id,
                    url,
                    title: title.as_ref().map(|v| v.as_str()).unwrap_or(url),
                    status: *status,
                    created_at: ago_timestamp(*created_at),
                    tags: Vec::from_iter(tags.iter().map(|v| v.name.clone())).join(", "),
                }
            },
        )
        .collect();
    IndexTemplate { pendings, health }.render_html()
}

pub async fn vite_counter() -> Result<impl IntoResponse, AppError> {
    ViteCounter {
        title: "Rust + Axum + Vite + Alpine",
    }
    .render_html()
}
