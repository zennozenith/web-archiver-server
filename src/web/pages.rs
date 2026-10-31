use askama::Template;
use axum::{
    Form,
    extract::{Path, State},
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    db::{
        self,
        tags::{TagRow, get_or_create_tag},
    },
    error::{AppError, Result},
    htmx::{HtmxResponse, PartialSwap},
    tag::{self, Name},
    template::FeedbackTemplate,
};

#[derive(Clone)]
pub(super) struct PageTag {
    pub id: i64,
    pub name: Name,
}

impl From<&TagRow> for PageTag {
    fn from(value: &TagRow) -> Self {
        Self {
            id: value.id,
            name: value.name.clone(),
        }
    }
}

pub(super) struct CaptureView {
    pub id: i64,
    pub version: i64,
    pub title: String,
    pub captured_at: String,
    pub status: String,
    pub has_html: bool,
    pub has_screenshot: bool,
}

#[derive(Template)]
#[template(path = "fragments/page-card.html")]
pub(super) struct PageCardTemplate {
    pub id: i64,
    pub url: String,
    pub host: String,
    pub title: String,
    pub search: String,
    pub last_captured: String,
    pub capture_count: i64,
    pub version_label: String,
    pub tags: Vec<PageTag>,
    pub captures: Vec<CaptureView>,
}

#[derive(Template)]
#[template(path = "fragments/page-tags.html")]
pub(super) struct PageTagsTemplate {
    pub page_id: i64,
    pub tags: Vec<PageTag>,
}

impl PageTagsTemplate {
    pub(super) async fn load(pool: &SqlitePool, page_id: i64) -> Result<Self> {
        let tags = db::pages::page_tags(pool, page_id).await?;
        Ok(Self {
            page_id,
            tags: tags.iter().map(PageTag::from).collect(),
        })
    }
}

#[derive(Deserialize)]
pub(super) struct TagForm {
    pub name: String,
}

pub(super) async fn add_page_tag(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
    Form(form): Form<TagForm>,
) -> Result<HtmxResponse> {
    let name = tag::clean_name(&form.name)?;
    if !db::pages::page_exists(&pool, id).await? {
        return Err(AppError::NotFound("page"));
    }

    let mut tx = pool.begin().await?;
    let tag_id = get_or_create_tag(&mut tx, &name).await?;
    sqlx::query("INSERT OR IGNORE INTO url_tag (url_id, tag_id) VALUES (?, ?)")
        .bind(id)
        .bind(tag_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let tags = PageTagsTemplate::load(&pool, id).await?;
    Ok(HtmxResponse::partials([PartialSwap::tpl(
        format!("#page-tags-{id}"),
        &tags,
    )?
    .swap("outerHTML")]))
}

pub(super) async fn remove_page_tag(
    State(pool): State<SqlitePool>,
    Path((id, tag_id)): Path<(i64, i64)>,
) -> Result<HtmxResponse> {
    sqlx::query("DELETE FROM url_tag WHERE url_id = ? AND tag_id = ?")
        .bind(id)
        .bind(tag_id)
        .execute(&pool)
        .await?;

    let tags = PageTagsTemplate::load(&pool, id).await?;
    Ok(HtmxResponse::partials([PartialSwap::tpl(
        format!("#page-tags-{id}"),
        &tags,
    )?
    .swap("outerHTML")]))
}

pub(super) async fn delete_page(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
) -> Result<HtmxResponse> {
    // Idempotent: the card should disappear even if the page is already gone.
    db::pages::delete_page(&pool, id).await?;

    Ok(HtmxResponse::partials([
        PartialSwap::new(format!("#page-{id}"), "").swap("outerHTML"),
        PartialSwap::tpl("#page-feedback", &FeedbackTemplate::success("Page deleted"))?
            .swap("innerHTML"),
    ]))
}
