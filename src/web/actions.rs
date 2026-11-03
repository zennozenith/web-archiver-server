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
    blob_store::BlobStore,
    db::{
        collection::{CollectionId, CollectionInput, CollectionRecord},
        page::{self, CreatePage, DeletePage, HighlightRecord, PageId, PageItem, UptatePage},
        tag::Tag,
    },
    error::{AppError, OptionExt, Result},
    helper::{NonEmptyString, parse_bool},
    htmx::HtmxRequest,
    image::{Image, ImageFormat},
    tag::{TagId, TagName, comma_separated_tags},
    template::RenderHtml,
    url_canon::canonicalize_url,
    web::{templates::PageCardTemplate, view::PageView},
};

// ───────────────────────────────────────────────────────────────────────────
// Helpers
// ───────────────────────────────────────────────────────────────────────────

fn parse_collection_id(value: Option<&String>) -> Option<CollectionId> {
    value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|id| *id > 0)
        .map(CollectionId::from)
}

fn parse_tag_id(value: &str) -> Result<TagId> {
    let id = value
        .trim()
        .parse::<i64>()
        .map_err(|_| AppError::NotFound("tag"))?;
    Ok(TagId::new(id))
}

async fn non_empty_parent(
    pool: &SqlitePool,
    id: Option<CollectionId>,
    current: Option<CollectionId>,
) -> Result<Option<CollectionId>> {
    match id {
        Some(parent)
            if Some(parent) != current && CollectionRecord::exists(pool, parent).await? =>
        {
            Ok(Some(parent))
        }
        _ => Ok(None),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Pages
// ───────────────────────────────────────────────────────────────────────────

/// Parsed from the multipart fields of POST /links
#[derive(Debug, Default)]
struct PageUpload {
    pub url: Option<String>,
    pub title: Option<NonEmptyString>,
    pub description: Option<String>,
    pub collection_id: Option<String>, // i64 string
    pub pinned: bool,
    pub tags: Vec<TagName>,
    pub thumbnail: Option<Image>,
}

async fn parse_multipart(mut mp: Multipart) -> Result<PageUpload> {
    let mut upload = PageUpload::default();
    let mut thumbnail: Option<Vec<u8>> = None;

    while let Some(field) = mp.next_field().await? {
        let name = field.name().unwrap_or("");
        match name {
            "url" => {
                let raw_url = NonEmptyString::non_empty(&field.text().await?);
                upload.url = raw_url;
            }
            "title" => upload.title = Some(NonEmptyString::trimmed(&field.text().await?)?),
            "description" => upload.description = NonEmptyString::non_empty(&field.text().await?),
            "collection_id" => upload.collection_id = Some(field.text().await?),
            "pinned" => upload.pinned = parse_bool(&field.text().await?)?,
            "tags" => {
                let t = field.text().await?;
                upload.tags = comma_separated_tags(Some(&t))?;
            }
            "thumbnail" => thumbnail = Some(field.bytes().await?.to_vec()),
            _ => {} // ignore unknown fields
        }
    }

    if let Some(bytes) = thumbnail {
        let format = ImageFormat::detect(&bytes).ok_or_else(|| {
            AppError::invalid("invalid_image_format", "cannot determine thumbnail format")
        })?;
        upload.thumbnail = Some(Image::new(format, bytes)?);
    }
    Ok(upload)
}

pub(crate) async fn create_link(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<Response> {
    let upload = parse_multipart(multipart?).await?;

    let raw_url = upload
        .url
        .ok_or_else(|| AppError::invalid("missing_url", "A link needs a URL."))?;
    let url = canonicalize_url(&raw_url)?;

    let title = upload
        .title
        .ok_or_else(|| AppError::invalid("missing_title", "A link needs a title."))?;
    let description = upload.description.unwrap_or_default();
    let collection_id = parse_collection_id(upload.collection_id.as_ref());
    let collection_id = non_empty_parent(&pool, collection_id, None).await?;
    let tags = upload.tags;
    let pinned = upload.pinned;
    let thumbnail = upload.thumbnail;

    let item = CreatePage {
        url,
        title,
        description,
        collection_id,
        pinned,
        tags,
        thumbnail,
    }
    .execute(&pool, &store)
    .await?;
    let link = PageView::from_item(&item, &[]);

    Ok(PageCardTemplate { link }.render_html()?.into_response())
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateLinkForm {
    name: Option<String>,
    collection_id: Option<String>,
    tags: Option<String>,
    description: Option<String>,
}

pub(crate) async fn update_link(
    State(pool): State<SqlitePool>,
    Path(id): Path<PageId>,
    Form(form): Form<UpdateLinkForm>,
) -> Result<Response> {
    let collection_id = parse_collection_id(form.collection_id.as_ref());
    let collection_id = non_empty_parent(&pool, collection_id, None).await?;
    let tags = comma_separated_tags(form.tags.as_ref())?;

    let updated = UptatePage::execute(
        &pool,
        id,
        form.name.as_deref(),
        form.description.as_deref().unwrap_or_default(),
        collection_id,
        &tags,
    )
    .await?;

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
    if DeletePage::execute(&pool, id).await? {
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
pub(crate) struct CollectionForm {
    name: String,
    description: Option<String>,
    parent_id: Option<String>,
    color: Option<String>,
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

    let parent_id = parse_collection_id(form.parent_id.as_ref());
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
    let parent_id = parse_collection_id(form.parent_id.as_ref());
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
pub(crate) struct SharingForm {
    public: Option<String>,
}

pub(crate) async fn update_sharing(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
    Form(form): Form<SharingForm>,
) -> Result<Response> {
    if CollectionRecord::set_public(&pool, id, form.public.is_some()).await? {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(AppError::NotFound("collection"))
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Tags
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
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
