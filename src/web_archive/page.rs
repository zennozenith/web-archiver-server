use std::str::FromStr;

use askama::Template;
use axum::{
    Form,
    extract::{Path, State},
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    db::{
        page::{self, PageId},
        tag::Tag,
    },
    error::{AppError, Result},
    htmx::{HtmxResponse, PartialSwap},
    tag::TagName,
};

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
#[template(path = "archive/_page_card.html")]
pub(super) struct PageCardTemplate {
    pub id: i64,
    pub url: String,
    pub host: String,
    pub display_title: String,
    pub search: String,
    pub pinned: bool,
    pub last_captured: String,
    pub capture_count: i64,
    pub version_label: String,
    pub tags: Vec<Tag>,
    pub captures: Vec<CaptureView>,
}

#[derive(Template)]
#[template(path = "archive/_page_tags.html")]
pub(super) struct PageTagsTemplate {
    pub page_id: i64,
    pub tags: Vec<Tag>,
}

impl PageTagsTemplate {
    pub(super) async fn load(pool: &SqlitePool, page_id: i64) -> Result<Self> {
        let tags = page::tags_of(pool, page_id.into()).await?;
        Ok(Self { page_id, tags })
    }
}

#[derive(Deserialize)]
pub(super) struct TagForm {
    pub name: String,
}

pub(super) async fn add_page_tag(
    State(pool): State<SqlitePool>,
    Path(id): Path<PageId>,
    Form(form): Form<TagForm>,
) -> Result<HtmxResponse> {
    let name = TagName::from_str(&form.name)?;

    if !page::exists(&pool, id).await? {
        return Err(AppError::NotFound("page"));
    }

    let mut tx = pool.begin().await?;
    let tag_id = Tag::get_or_create_id(&mut *tx, &name).await?;
    sqlx::query("INSERT OR IGNORE INTO page_tag (page_id, tag_id) VALUES (?, ?)")
        .bind(id)
        .bind(tag_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let tags = PageTagsTemplate::load(&pool, id.get()).await?;
    Ok(HtmxResponse::partials([PartialSwap::tpl(
        format!("#page-tags-{id}"),
        &tags,
    )?
    .swap("outerHTML")]))
}
