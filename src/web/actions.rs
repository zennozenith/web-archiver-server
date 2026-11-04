//! Mutation handlers for links, collections and tags.

use axum::{
    Form,
    extract::{Multipart, Path, State, multipart::MultipartRejection},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    api::parse_multipart,
    blob_store::BlobStore,
    db::{
        collection::{CollectionId, CollectionInput, CollectionRecord, non_empty_parent},
        page::{self, CreatePage, HighlightRecord, PageId, PageItem, UpdatePage},
        tag::Tag,
    },
    error::{AppError, OptionExt, Result},
    htmx::HtmxRequest,
    tag::{TagId, TagName},
    template::RenderHtml,
    web::{templates::PageCardTemplate, view::PageView},
};

// ───────────────────────────────────────────────────────────────────────────
// Helpers
// ───────────────────────────────────────────────────────────────────────────

fn parse_tag_id(value: &str) -> Result<TagId> {
    let id = value
        .trim()
        .parse::<i64>()
        .map_err(|_| AppError::NotFound("tag"))?;
    Ok(TagId::new(id))
}

// ───────────────────────────────────────────────────────────────────────────
// Pages
// ───────────────────────────────────────────────────────────────────────────

pub(crate) async fn create_link(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<Response> {
    let upload = parse_multipart(multipart?).await?;

    let collection_id = non_empty_parent(&pool, upload.collection_id, None).await?;

    let item = CreatePage {
        collection_id,
        ..upload
    }
    .execute(&pool, &store)
    .await?;

    let link = PageView::from_item(&item, &[]);

    Ok(PageCardTemplate { link }.render_html()?.into_response())
}

pub(crate) async fn update_link(
    State(pool): State<SqlitePool>,
    Path(id): Path<PageId>,
    Form(form): Form<UpdatePage>,
) -> Result<Response> {
    let updated = UpdatePage::apply(form, &pool, id).await?;

    if updated {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(AppError::NotFound("link"))
    }
}

pub(crate) async fn delete_link(
    State(pool): State<SqlitePool>,
    Path(id): Path<PageId>,
) -> Result<Response> {
    if PageId::delete(id, &pool).await? {
        // A 200 with an empty body makes htmx remove the card via outerHTML.
        Ok(StatusCode::OK.into_response())
    } else {
        Err(AppError::NotFound("link"))
    }
}

pub(crate) async fn toggle_pin(
    State(pool): State<SqlitePool>,
    Path(id): Path<PageId>,
) -> Result<Response> {
    match page::toggle_pinned(&pool, id).await? {
        Some(_) => Ok(StatusCode::NO_CONTENT.into_response()),
        None => Err(AppError::NotFound("link")),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HighlightForm {
    color: Option<String>,
    text: Option<String>,
}

pub(crate) async fn add_highlight(
    State(pool): State<SqlitePool>,
    Path(id): Path<PageId>,
    Form(form): Form<HighlightForm>,
) -> Result<Response> {
    let color = form.color.unwrap_or_else(|| "yellow".to_owned());
    let text = form
        .text
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "Highlighted passage".to_owned());

    if PageItem::load(&pool, id).await?.is_none() {
        return Err(AppError::NotFound("link"));
    }

    HighlightRecord::add(&pool, id, &text, &color).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ───────────────────────────────────────────────────────────────────────────
// Collections
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CollectionForm {
    name: String,
    description: Option<String>,
    parent_id: Option<String>,
    color: Option<String>,
    // TODO: emoji
}

fn collection_input(form: &CollectionForm, parent_id: Option<CollectionId>) -> CollectionInput<'_> {
    CollectionInput {
        name: form.name.trim(),
        emoji: "📁",
        description: form.description.as_deref().unwrap_or_default(),
        color: form.color.as_deref().unwrap_or("#6d5dfc"),
        parent_id,
        public: false,
    }
}

pub(crate) async fn create_collection(
    State(pool): State<SqlitePool>,
    Form(form): Form<CollectionForm>,
) -> Result<Response> {
    if form.name.trim().is_empty() {
        return Err(AppError::invalid(
            "missing_name",
            "A collection needs a name.",
        ));
    }

    let parent_id = CollectionId::parse_or(form.parent_id.as_ref());
    let parent_id = non_empty_parent(&pool, parent_id, None).await?;
    CollectionRecord::create(&pool, &collection_input(&form, parent_id)).await?;

    Ok(Redirect::to("/collections").into_response())
}

fn collection_update_input(
    form: &CollectionForm,
    parent_id: Option<CollectionId>,
    public: bool,
) -> CollectionInput<'_> {
    CollectionInput {
        public,
        ..collection_input(form, parent_id)
    }
}

pub(crate) async fn update_collection(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
    htmx: HtmxRequest,
    Form(form): Form<CollectionForm>,
) -> Result<Response> {
    if form.name.trim().is_empty() {
        return Err(AppError::invalid(
            "missing_name",
            "A collection needs a name.",
        ));
    }

    let current = CollectionRecord::load(&pool, id)
        .await?
        .or_not_found("collection")?;
    let parent_id = CollectionId::parse_or(form.parent_id.as_ref());
    let parent_id = non_empty_parent(&pool, parent_id, Some(id)).await?;

    let updated = CollectionRecord::update(
        &pool,
        id,
        &collection_update_input(&form, parent_id, current.public),
    )
    .await?;

    if updated.is_none() {
        return Err(AppError::NotFound("collection"));
    }

    if htmx.is_htmx {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(Redirect::to(&format!("/collections/{id}")).into_response())
    }
}

pub(crate) async fn delete_collection(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
) -> Result<Response> {
    if CollectionRecord::delete(&pool, id).await? {
        Ok(StatusCode::OK.into_response())
    } else {
        Err(AppError::NotFound("collection"))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SharingForm {
    public: Option<bool>,
}

pub(crate) async fn update_sharing(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
    Form(form): Form<SharingForm>,
) -> Result<Response> {
    if CollectionRecord::set_public(&pool, id, form.public.unwrap_or(false)).await? {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(AppError::NotFound("collection"))
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Tags
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagForm {
    name: String,
}

pub(crate) async fn rename_tag(
    State(pool): State<SqlitePool>,
    Path(key): Path<String>,
    Form(form): Form<TagForm>,
) -> Result<Response> {
    let id = parse_tag_id(&key)?;
    let name: TagName = form.name.parse()?;

    if let Some(existing_id) = Tag::find_id_by_name(&pool, &name, Some(id)).await? {
        return Err(AppError::Duplicate {
            existing_id: existing_id.get(),
            msg: "Tag already exist",
        });
    }

    if Tag::rename(&pool, id, &name).await?.is_none() {
        return Err(AppError::NotFound("tag"));
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

pub(crate) async fn delete_tag(
    State(pool): State<SqlitePool>,
    Path(key): Path<String>,
) -> Result<Response> {
    let id = parse_tag_id(&key)?;

    if Tag::delete(&pool, id).await? {
        Ok(StatusCode::OK.into_response())
    } else {
        Err(AppError::NotFound("tag"))
    }
}
