use askama::Template;
use axum::{
    Form,
    extract::{Path, State},
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    db,
    error::{AppError, Result},
    htmx::{HtmxResponse, PartialSwap},
    tag::clean_tag_name,
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

/// Colors are palette keys like "red" or "deep-blue", never hex values.
fn clean_color(color: Option<&str>) -> Result<Option<String>> {
    let Some(color) = color.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    let ok = color.len() <= 32
        && color
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(AppError::invalid(
            "invalid_color",
            "color must be a palette key: lowercase letters, digits and '-' (max 32)",
        ));
    }
    Ok(Some(color.to_owned()))
}

async fn tags_response(pool: &SqlitePool, message: &'static str) -> Result<HtmxResponse> {
    let tags = db::pages::list_tag_counts(pool)
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
    let name = clean_tag_name(&form.name)?;
    let color = clean_color(form.color.as_deref())?;

    if db::pages::create_tag(&pool, name.as_str(), color.as_deref())
        .await?
        .is_none()
    {
        let existing_id = db::pages::tag_name_exists(&pool, name.as_str(), None)
            .await?
            .unwrap_or(0);
        return Err(AppError::DuplicateTag { existing_id });
    }

    tags_response(&pool, "Tag added").await
}

pub(super) async fn rename_tag(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
    Form(form): Form<RenameTagForm>,
) -> Result<HtmxResponse> {
    let name = clean_tag_name(&form.name)?;

    if let Some(existing_id) = db::pages::tag_name_exists(&pool, name.as_str(), Some(id)).await? {
        return Err(AppError::DuplicateTag { existing_id });
    }
    if db::pages::rename_tag(&pool, id, name.as_str())
        .await?
        .is_none()
    {
        return Err(AppError::NotFound("tag"));
    }

    tags_response(&pool, "Tag renamed").await
}

pub(super) async fn delete_tag(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
) -> Result<HtmxResponse> {
    if !db::pages::delete_tag(&pool, id).await? {
        return Err(AppError::NotFound("tag"));
    }
    tags_response(&pool, "Tag deleted").await
}
