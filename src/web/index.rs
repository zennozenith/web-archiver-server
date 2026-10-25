use crate::{db, dto, error::Result, template::RenderHtml};

use super::pending::PendingFragmentTemplate;

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
#[template(path = "vite-counter.html")]
struct ViteCounter<'a> {
    title: &'a str,
}

pub async fn index(State(pool): State<SqlitePool>) -> Result<impl IntoResponse> {
    // TODO:
    const LIMIT: u32 = 5;

    // OPTIMIZE: join both database calls
    let health = db::health::health(&pool).await?;

    let dto::Page { items, .. } = db::pending::list_pending(&pool, None, None, LIMIT).await?;
    let pendings = items.iter().map(PendingFragmentTemplate::from).collect();
    IndexTemplate { pendings, health }.render_html()
}

pub async fn vite_counter() -> Result<impl IntoResponse> {
    ViteCounter {
        title: "Rust + Axum + Vite + Alpine",
    }
    .render_html()
}
