use askama::Template;
use axum::{
    Form,
    extract::{Path, State},
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    db::{
        helpers::Inserted,
        tags::{TagRow, list_tag_counts},
    },
    error::{AppError, Result},
    htmx::{HtmxResponse, PartialSwap},
    tag,
    template::FeedbackTemplate,
};

#[derive(Clone)]
pub(super) struct TagView {
    pub id: i64,
    pub name: String,
    pub pages: i64,
    pub pending: i64,
}

#[derive(Template)]
#[template(path = "fragments/tag-list.html")]
pub(super) struct TagListTemplate {
    pub tags: Vec<TagView>,
}

#[derive(Deserialize)]
pub(super) struct CreateTagForm {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RenameTagForm {
    pub name: String,
}

async fn tags_response(pool: &SqlitePool, message: &'static str) -> Result<HtmxResponse> {
    let tags = list_tag_counts(pool)
        .await?
        .into_iter()
        .map(|t| TagView {
            id: t.id,
            name: t.name,
            pages: t.pages,
            pending: t.pending,
        })
        .collect();

    Ok(HtmxResponse::partials([
        PartialSwap::tpl("#tags-list", &TagListTemplate { tags })?.swap("innerHTML"),
        PartialSwap::tpl("#tags-feedback", &FeedbackTemplate::success(message))?.swap("innerHTML"),
    ]))
}

pub(super) async fn create_tag(
    State(pool): State<SqlitePool>,
    Form(form): Form<CreateTagForm>,
) -> Result<HtmxResponse> {
    let name = tag::clean_name(&form.name)?;
    let color = tag::validate_color(form.color.as_deref())?;

    if let Inserted::Existing(existing_id) = TagRow::create(&pool, name, color).await? {
        return Err(AppError::DuplicateTag { existing_id });
    }

    tags_response(&pool, "Tag added").await
}

pub(super) async fn rename_tag(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
    Form(form): Form<RenameTagForm>,
) -> Result<HtmxResponse> {
    let name = tag::clean_name(&form.name)?;

    if let Some(existing_id) = TagRow::exists(&pool, &name, Some(id)).await? {
        return Err(AppError::DuplicateTag { existing_id });
    }
    if TagRow::rename(&pool, id, &name).await?.is_none() {
        return Err(AppError::NotFound("tag"));
    }

    tags_response(&pool, "Tag renamed").await
}

pub(super) async fn delete_tag(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
) -> Result<HtmxResponse> {
    if TagRow::delete(&pool, id).await? == 0 {
        return Err(AppError::NotFound("tag"));
    }
    tags_response(&pool, "Tag deleted").await
}
