//! Mutation handlers for links, collections and tags.

use axum::{
    Form,
    extract::{Multipart, Path, Query, State, multipart::MultipartRejection},
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
        helper::{CreateOutcome, FieldPatch},
        page::{self, CreatePage, HighlightRecord, PageId, PageItem, UpdatePage},
        pending::{PendingId, PendingRecord, PendingStatus, PendingUpdate},
        tag::Tag,
    },
    error::{AppError, OptionExt, Result},
    htmx::HtmxRequest,
    image::{Image, ImageFormat},
    tag::{TagId, TagName, comma_separated_tags, parse_names},
    template::RenderHtml,
    url_canon::canonicalize_url,
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

fn image_from_bytes(bytes: Vec<u8>) -> Result<Option<Image>> {
    if bytes.is_empty() {
        return Ok(None);
    }

    let format = ImageFormat::detect(&bytes).ok_or_else(|| {
        AppError::invalid("invalid_image_format", "cannot determine thumbnail format")
    })?;
    Ok(Some(Image::new(format, bytes)?))
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

#[derive(Default)]
struct LinkEditForm {
    display_title: Option<String>,
    description: Option<String>,
    collection_id: Option<String>,
    tags: Option<String>,
    thumbnail: Option<Image>,
}

pub(crate) async fn update_link(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    Path(id): Path<PageId>,
    htmx: HtmxRequest,
    mut multipart: Multipart,
) -> Result<Response> {
    let mut form = LinkEditForm::default();

    while let Some(field) = multipart.next_field().await? {
        match field.name().unwrap_or("") {
            "displayTitle" => form.display_title = Some(field.text().await?),
            "description" => form.description = Some(field.text().await?),
            "collectionId" => form.collection_id = Some(field.text().await?),
            "tags" => form.tags = Some(field.text().await?),
            "thumbnail" => {
                form.thumbnail = image_from_bytes(field.bytes().await?.to_vec())?;
            }
            _ => {}
        }
    }

    let update = UpdatePage {
        display_title: form
            .display_title
            .map_or(FieldPatch::Unchanged, FieldPatch::Set),
        description: form
            .description
            .map_or(FieldPatch::Unchanged, FieldPatch::Set),
        collection_id: form
            .collection_id
            .map_or(FieldPatch::Unchanged, FieldPatch::Set),
        tags: form.tags.map_or(FieldPatch::Unchanged, FieldPatch::Set),
        ..UpdatePage::default()
    };

    if !UpdatePage::apply(update, &pool, id).await? {
        return Err(AppError::NotFound("link"));
    }

    if let Some(thumbnail) = form.thumbnail {
        PageId::set_thumbnail(id, &pool, &store, Some(thumbnail)).await?;
    }

    if htmx.is_htmx {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(Redirect::to(&format!("/links/{id}")).into_response())
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
    emoji: Option<String>,
    description: Option<String>,
    parent_id: Option<String>,
    color: Option<String>,
}

fn collection_input(form: &CollectionForm, parent_id: Option<CollectionId>) -> CollectionInput<'_> {
    let emoji = form
        .emoji
        .as_deref()
        .map(str::trim)
        .filter(|emoji| !emoji.is_empty())
        .unwrap_or("📁");

    CollectionInput {
        name: form.name.trim(),
        emoji,
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

// ───────────────────────────────────────────────────────────────────────────
// Pending queue
// ───────────────────────────────────────────────────────────────────────────

const MAX_BULK_PENDING: usize = 500;

#[derive(Debug, Deserialize)]
pub(crate) struct PendingForm {
    url: String,
    tags: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BulkPendingForm {
    items: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingUpdateForm {
    url: Option<String>,
    status: Option<PendingStatus>,
    tags: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClearPendingQuery {
    status: PendingStatus,
}

/// Split one bulk-entry line into its URL and raw tag names.
///
/// `URL | tag1, tag2`, `URL tag1, tag2` and a bare URL are all accepted.
fn parse_bulk_line(line: &str) -> (String, Vec<String>) {
    let line = line.trim();
    let (url, tags) = if let Some((url, tags)) = line.split_once('|') {
        (url.trim(), tags.trim())
    } else if let Some((url, tags)) = line.split_once(char::is_whitespace) {
        (url, tags.trim())
    } else {
        (line, "")
    };

    let tags = tags
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect();

    (url.to_owned(), tags)
}

fn parse_bulk_entries(raw: &str) -> Vec<(String, Vec<String>)> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(parse_bulk_line)
        .collect()
}

async fn pending_conflict_or_not_found(
    pool: &SqlitePool,
    id: PendingId,
    conflict_message: &str,
) -> Result<AppError> {
    Ok(if PendingRecord::exists(pool, id).await? {
        AppError::Conflict(conflict_message.to_owned())
    } else {
        AppError::NotFound("pending item")
    })
}

pub(crate) async fn create_pending(
    State(pool): State<SqlitePool>,
    Form(form): Form<PendingForm>,
) -> Result<Response> {
    let url = canonicalize_url(&form.url)?;
    let tags = comma_separated_tags(form.tags.as_ref())?;

    let mut tx = pool.begin().await?;
    PendingRecord::enqueue(&mut tx, &url, &tags).await?;
    tx.commit().await?;

    Ok(Redirect::to("/pending").into_response())
}

pub(crate) async fn bulk_create_pending(
    State(pool): State<SqlitePool>,
    Form(form): Form<BulkPendingForm>,
) -> Result<Response> {
    let entries = parse_bulk_entries(&form.items);
    if entries.is_empty() || entries.len() > MAX_BULK_PENDING {
        return Err(AppError::BadRequest(format!(
            "add between 1 and {MAX_BULK_PENDING} URLs"
        )));
    }

    let mut added = 0_u64;
    let mut duplicates = 0_u64;
    let mut invalid = 0_u64;

    let mut tx = pool.begin().await?;
    for (raw_url, raw_tags) in entries {
        let parsed = canonicalize_url(&raw_url).and_then(|url| Ok((url, parse_names(&raw_tags)?)));
        let Ok((url, tags)) = parsed else {
            invalid = invalid.saturating_add(1);
            continue;
        };

        match PendingRecord::enqueue(&mut tx, &url, &tags).await? {
            CreateOutcome::Created(_) => added = added.saturating_add(1),
            CreateOutcome::Existing(_) => duplicates = duplicates.saturating_add(1),
        }
    }
    tx.commit().await?;

    Ok(Redirect::to(&format!(
        "/pending?added={added}&duplicates={duplicates}&invalid={invalid}"
    ))
    .into_response())
}

pub(crate) async fn update_pending(
    State(pool): State<SqlitePool>,
    Path(id): Path<PendingId>,
    htmx: HtmxRequest,
    Form(form): Form<PendingUpdateForm>,
) -> Result<Response> {
    let tags = match form.tags.as_ref() {
        Some(raw) => Some(comma_separated_tags(Some(raw))?),
        None => None,
    };
    let url = form
        .url
        .as_deref()
        .map(canonicalize_url)
        .transpose()?
        .map(|url| url.url);

    let mut tx = pool.begin().await?;
    let current = PendingRecord::status_of(&mut *tx, id)
        .await?
        .ok_or(AppError::NotFound("pending item"))?;

    if form.status == Some(PendingStatus::InProgress) {
        return Err(AppError::invalid(
            "invalid_status",
            "items are put in progress by the archive worker",
        ));
    }
    if form.status.is_some() && current == PendingStatus::InProgress {
        return Err(AppError::Conflict(
            "item is currently being archived".to_owned(),
        ));
    }

    PendingUpdate {
        status: form.status,
        url,
        tags: tags.as_deref(),
    }
    .apply(&mut tx, id)
    .await?;

    tx.commit().await?;

    if htmx.is_htmx {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(Redirect::to("/pending").into_response())
    }
}

pub(crate) async fn retry_pending(
    State(pool): State<SqlitePool>,
    Path(id): Path<PendingId>,
) -> Result<Response> {
    if PendingRecord::retry(&pool, id).await? {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(pending_conflict_or_not_found(&pool, id, "only failed items can be retried").await?)
    }
}

pub(crate) async fn delete_pending(
    State(pool): State<SqlitePool>,
    Path(id): Path<PendingId>,
) -> Result<Response> {
    if PendingRecord::delete(&pool, id).await? {
        // A 200 with an empty body makes htmx remove the row via outerHTML.
        Ok(StatusCode::OK.into_response())
    } else {
        Err(pending_conflict_or_not_found(&pool, id, "item is currently being archived").await?)
    }
}

pub(crate) async fn clear_pending(
    State(pool): State<SqlitePool>,
    Query(query): Query<ClearPendingQuery>,
    htmx: HtmxRequest,
) -> Result<Response> {
    if query.status == PendingStatus::InProgress {
        return Err(AppError::BadRequest(
            "cannot clear in_progress items".to_owned(),
        ));
    }

    PendingRecord::delete_all_with_status(&pool, query.status).await?;

    if htmx.is_htmx {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(Redirect::to("/pending").into_response())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_bulk_entries;

    #[test]
    fn bulk_lines_accept_optional_tags() {
        assert_eq!(
            parse_bulk_entries("https://example.com\n"),
            vec![("https://example.com".to_owned(), Vec::new())]
        );
        assert_eq!(
            parse_bulk_entries("https://example.com | design, reading\n"),
            vec![(
                "https://example.com".to_owned(),
                vec!["design".to_owned(), "reading".to_owned()]
            )]
        );
        assert_eq!(
            parse_bulk_entries("https://example.com design reading\n"),
            vec![(
                "https://example.com".to_owned(),
                vec!["design".to_owned(), "reading".to_owned()]
            )]
        );
    }

    #[test]
    fn bulk_lines_skip_blanks_and_comments() {
        assert_eq!(
            parse_bulk_entries("\n# comment\nhttps://example.com\n"),
            vec![("https://example.com".to_owned(), Vec::new())]
        );
    }
}
