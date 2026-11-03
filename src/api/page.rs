use axum::{extract::State, http::StatusCode};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    api::tag::CreateTagRequest,
    db::{
        collection::CollectionId,
        helper::CursorPage,
        page::{
            self, DeletePage, PageCursor, PageDetail, PageField, PageFilter, PageId, PageItem,
            PageSort,
        },
        tag::Tag,
    },
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath, ApiQuery},
    limit::Limit,
    tag::{TagId, TagName, parse_names},
    url_canon::canonicalize_url,
};

const PAGE: &str = "page";
const DEFAULT_LIMIT: Limit = Limit::from_const(50);

#[derive(Debug, Deserialize)]
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
        tag_id: query.tag_id,
        pinned_only: query.pinned_only.unwrap_or_default(),
        limit: query.limit.unwrap_or(DEFAULT_LIMIT),
        cursor: query.cursor.map(PageCursor::try_from).transpose()?,
    };

    Ok(ApiJson(PageItem::list(&pool, &filter).await?))
}

// ── GET /api/page/lookup?url=... ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
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

// ── DELETE /api/page/{id} ───────────────────────────────────────────────────
// Orphaned content rows are left for POST /api/maintenance/gc.

pub(super) async fn delete_page(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PageId>,
) -> Result<StatusCode> {
    if !DeletePage::execute(&pool, id).await? {
        return Err(AppError::NotFound(PAGE));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── GET /api/page/{id}/captures ─────────────────────────────────────────────

// #[derive(Debug, Deserialize)]
// pub(super) struct ListCapturesQuery {
//     limit: Option<Limit>,
//     /// Version of the last capture already seen.
//     cursor: Option<Cursor>,
// }

// pub(super) async fn list_page_captures(
//     State(pool): State<SqlitePool>,
//     ApiPath(id): ApiPath<PageId>,
//     ApiQuery(query): ApiQuery<ListCapturesQuery>,
// ) -> Result<ApiJson<CursorPage<CaptureSummary>>> {
//     if !pages::exists(&pool, id).await? {
//         return Err(AppError::NotFound(PAGE));
//     }

//     let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
//     let captures = CaptureSummary::list(&pool, id, query.cursor, limit).await?;
//     Ok(ApiJson(captures))
// }

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
