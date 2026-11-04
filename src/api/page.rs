use axum::{
    extract::{Multipart, State, multipart::MultipartRejection},
    http::StatusCode,
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    api::tag::CreateTagRequest,
    blob_store::BlobStore,
    db::{
        capture::CaptureSummary,
        collection::{CollectionId, non_empty_parent},
        helper::{Cursor, CursorPage},
        page::{
            self, CreatePage, PageCursor, PageDetail, PageField, PageFilter, PageId, PageItem,
            PageSort, UpdatePage,
        },
        tag::Tag,
    },
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath, ApiQuery},
    helper::{NonEmptyString, parse_bool},
    image::{Image, ImageFormat},
    limit::Limit,
    tag::{TagId, TagName, comma_separated_tags, parse_names},
    url_canon::canonicalize_url,
};

const PAGE: &str = "page";
const DEFAULT_LIMIT: Limit = Limit::from_const(50);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReplaceTagsRequest {
    pub tags: Vec<CreateTagRequest>,
}

async fn load_detail(pool: &SqlitePool, id: PageId) -> Result<ApiJson<PageDetail>> {
    let detail = PageDetail::load(pool, id).await?.or_not_found(PAGE)?;
    Ok(ApiJson(detail))
}

// ── GET /api/page ───────────────────────────────────────────────────────────
// Most recently archived first; `cursor` comes from the previous page's `nextCursor`.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageQuery {
    pub text: Option<String>,
    pub field: Option<PageField>,
    pub sort: Option<PageSort>,
    pub pinned_only: Option<bool>,
    pub collection_id: Option<CollectionId>,
    pub limit: Option<Limit>,
    pub tag_id: Option<TagId>,
    pub cursor: Option<String>,
}

pub(super) async fn list_pages(
    State(pool): State<SqlitePool>,
    ApiQuery(query): ApiQuery<PageQuery>,
) -> Result<ApiJson<CursorPage<PageItem, PageCursor>>> {
    let filter = PageFilter {
        text: query.text,
        field: query.field.unwrap_or_default(),
        sort: query.sort.unwrap_or_default(),
        collection_id: query.collection_id,
        unorganized_only: false,
        tag_id: query.tag_id,
        pinned_only: query.pinned_only.unwrap_or_default(),
        limit: query.limit.unwrap_or(DEFAULT_LIMIT),
        cursor: query.cursor.map(PageCursor::try_from).transpose()?,
    };

    Ok(ApiJson(PageItem::list(&pool, &filter).await?))
}

// ── POST /api/page ──────────────────────────────────────────────────────

pub(crate) async fn create_page(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<ApiJson<PageItem>> {
    let upload = parse_multipart(multipart?).await?;

    let collection_id = non_empty_parent(&pool, upload.collection_id, None).await?;

    let item = CreatePage {
        collection_id,
        ..upload
    }
    .execute(&pool, &store)
    .await?;

    Ok(ApiJson(item))
}

// ── GET /api/page/lookup?url=... ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LookupPageQuery {
    url: String,
}

pub(super) async fn lookup_page(
    State(pool): State<SqlitePool>,
    ApiQuery(query): ApiQuery<LookupPageQuery>,
) -> Result<ApiJson<PageDetail>> {
    let url = canonicalize_url(&query.url)?;
    let detail = PageDetail::load_by_url(&pool, &url)
        .await?
        .or_not_found(PAGE)?;
    Ok(ApiJson(detail))
}

// ── GET /api/page/{id} ──────────────────────────────────────────────────────

pub(super) async fn get_page(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PageId>,
) -> Result<ApiJson<PageDetail>> {
    load_detail(&pool, id).await
}

// ── PATCH /api/page/{id} ──────────────────────────────────────────────────────

pub(super) async fn patch_page(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PageId>,
    ApiJson(page): ApiJson<UpdatePage>,
) -> Result<StatusCode> {
    if page.apply(&pool, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound("page"))
    }
}

// ── DELETE /api/page/{id} ───────────────────────────────────────────────────
// Orphaned content rows are left for POST /api/maintenance/gc.

pub(super) async fn delete_page(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PageId>,
) -> Result<StatusCode> {
    if !PageId::delete(id, &pool).await? {
        return Err(AppError::NotFound(PAGE));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── GET /api/page/{id}/captures ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListCapturesQuery {
    limit: Option<Limit>,
    /// Version of the last capture already seen.
    cursor: Option<Cursor>,
}

pub(super) async fn list_page_captures(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PageId>,
    ApiQuery(query): ApiQuery<ListCapturesQuery>,
) -> Result<ApiJson<CursorPage<CaptureSummary>>> {
    if !page::exists(&pool, id).await? {
        return Err(AppError::NotFound(PAGE));
    }

    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
    let captures = CaptureSummary::list(&pool, id, query.cursor, limit).await?;
    Ok(ApiJson(captures))
}

// ── Tags on a page ──────────────────────────────────────────────────────────

// PUT /api/page/{id}/tags   { "tags": [{ "name": "..." }, ...] }  replaces the full set
pub(super) async fn put_page_tags(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PageId>,
    ApiJson(req): ApiJson<ReplaceTagsRequest>,
) -> Result<ApiJson<Vec<Tag>>> {
    // TODO: also apply each entry's color.
    let raw_names: Vec<String> = req.tags.into_iter().map(|tag| tag.name).collect();
    let names = parse_names(&raw_names)?;

    let tags = page::put_tags(&pool, id, &names)
        .await?
        .or_not_found(PAGE)?;
    Ok(ApiJson(tags))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AddPageTagRequest {
    name: String,
}

// POST /api/page/{id}/tag   { "name": "..." }  get-or-create + attach
pub(super) async fn add_page_tag(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PageId>,
    ApiJson(req): ApiJson<AddPageTagRequest>,
) -> Result<ApiJson<Tag>> {
    let name: TagName = req.name.parse()?;

    let tag = page::add_tag(&pool, id, &name).await?.or_not_found(PAGE)?;
    Ok(ApiJson(tag))
}

// DELETE /api/page/{id}/tag/{tag_id}
pub(super) async fn remove_page_tag(
    State(pool): State<SqlitePool>,
    ApiPath((page_id, tag_id)): ApiPath<(PageId, TagId)>,
) -> Result<StatusCode> {
    if !page::remove_tag(&pool, page_id, tag_id).await? {
        return Err(AppError::NotFound("tag on page"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// Parsing create

pub(crate) async fn parse_multipart(mut mp: Multipart) -> Result<CreatePage> {
    // Parsed from the multipart fields of POST
    let mut upload_url: Option<String> = None;
    let mut upload_display_title: Option<NonEmptyString> = None;
    let mut upload_description: Option<String> = None;
    let mut upload_collection_id: Option<String> = None; // i64 string
    let mut upload_pinned: bool = false;
    let mut upload_tags: Vec<TagName> = Vec::new();
    let mut upload_thumbnail: Option<Image> = None;
    let mut upload_thumbnail_bytes: Option<Vec<u8>> = None;

    while let Some(field) = mp.next_field().await? {
        let name = field.name().unwrap_or("");
        match name {
            "url" => {
                let raw_url = NonEmptyString::non_empty(&field.text().await?);
                upload_url = raw_url;
            }
            "displayTitle" => {
                upload_display_title = Some(NonEmptyString::trimmed(&field.text().await?)?);
            }
            "description" => upload_description = NonEmptyString::non_empty(&field.text().await?),
            "collectionId" => upload_collection_id = Some(field.text().await?),
            "pinned" => upload_pinned = parse_bool(&field.text().await?)?,
            "tags" => {
                let t = field.text().await?;
                upload_tags = comma_separated_tags(Some(&t))?;
            }
            "thumbnail" => {
                let bytes = field.bytes().await?.to_vec();
                // Browsers submit an empty part when no file was chosen.
                upload_thumbnail_bytes = (!bytes.is_empty()).then_some(bytes);
            }
            _ => {} // ignore unknown fields
        }
    }

    if let Some(bytes) = upload_thumbnail_bytes {
        let format = ImageFormat::detect(&bytes).ok_or_else(|| {
            AppError::invalid("invalid_image_format", "cannot determine thumbnail format")
        })?;
        upload_thumbnail = Some(Image::new(format, bytes)?);
    }

    let raw_url =
        upload_url.ok_or_else(|| AppError::invalid("missing_url", "A page needs a URL."))?;
    let url = canonicalize_url(&raw_url)?;

    let display_title = upload_display_title.ok_or_else(|| {
        AppError::invalid("missing_display_title", "A page needs a display title.")
    })?;
    let description = upload_description.unwrap_or_default();
    let collection_id = CollectionId::parse_or(upload_collection_id.as_ref());
    let pinned = upload_pinned;
    let tags = upload_tags;
    let thumbnail = upload_thumbnail;

    Ok(CreatePage {
        url,
        display_title,
        description,
        collection_id,
        pinned,
        tags,
        thumbnail,
    })
}
