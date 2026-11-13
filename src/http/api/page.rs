//! Pages (saved links): listing, creation, edits and tags.

use axum::{
    Router,
    extract::{DefaultBodyLimit, Multipart, State, multipart::MultipartRejection},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    auth::AuthUser,
    db::{
        capture::CaptureSummary,
        collection::CollectionId,
        helper::{Cursor, CursorPage},
        page::{PageCursor, PageDetail, PageField, PageId, PageItem, PageSort, UpdatePage},
        tag::Tag,
    },
    error::Result,
    http::{
        extract::body::{AppJson, AppPath, AppQuery},
        multipart::parse_page_upload,
    },
    infra::{TagId, TagName},
    service,
    util::limit::Limit,
};

const DEFAULT_LIMIT: Limit = Limit::from_const(50);
const ARCHIVE_BODY_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageQuery {
    text: Option<String>,
    field: Option<PageField>,
    sort: Option<PageSort>,
    pinned_only: Option<bool>,
    collection_id: Option<CollectionId>,
    unorganized_only: Option<bool>,
    limit: Option<Limit>,
    tag_id: Option<TagId>,
    cursor: Option<String>,
}

async fn list_pages(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<PageQuery>,
) -> Result<AppJson<CursorPage<PageItem, PageCursor>>> {
    let query = service::page::PageListQuery {
        text: query.text,
        field: query.field.unwrap_or_default(),
        sort: query.sort.unwrap_or_default(),
        collection_id: query.collection_id,
        unorganized_only: query.unorganized_only.unwrap_or(false),
        tag_id: query.tag_id,
        pinned_only: query.pinned_only.unwrap_or(false),
        limit: query.limit.unwrap_or(DEFAULT_LIMIT),
        cursor: query.cursor.as_deref().map(str::parse).transpose()?,
    };
    Ok(AppJson(service::page::list(&ctx, &user, &query).await?))
}

async fn create_page(
    State(ctx): State<AppState>,
    user: AuthUser,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<AppJson<PageItem>> {
    let upload = parse_page_upload(multipart?).await?;
    Ok(AppJson(service::page::create(&ctx, &user, upload).await?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LookupPageQuery {
    url: String,
}

async fn lookup_page(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<LookupPageQuery>,
) -> Result<AppJson<PageDetail>> {
    Ok(AppJson(
        service::page::lookup(&ctx, &user, &query.url).await?,
    ))
}

async fn get_page(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
) -> Result<AppJson<PageDetail>> {
    Ok(AppJson(service::page::get(&ctx, &user, id).await?))
}

async fn patch_page(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
    AppJson(page): AppJson<UpdatePage>,
) -> Result<StatusCode> {
    service::page::patch(&ctx, &user, id, page).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_page(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
) -> Result<StatusCode> {
    service::page::delete(&ctx, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListCapturesQuery {
    limit: Option<Limit>,
    cursor: Option<Cursor>,
}

async fn list_page_captures(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
    AppQuery(query): AppQuery<ListCapturesQuery>,
) -> Result<AppJson<CursorPage<CaptureSummary>>> {
    Ok(AppJson(
        service::page::captures(&ctx, &user, id, query.cursor, query.limit).await?,
    ))
}

#[derive(Debug, Deserialize)]
struct TagNameOnly {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ReplaceTagsRequest {
    tags: Vec<TagNameOnly>,
}

async fn put_page_tags(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
    AppJson(req): AppJson<ReplaceTagsRequest>,
) -> Result<AppJson<Vec<Tag>>> {
    let raw: Vec<String> = req.tags.into_iter().map(|tag| tag.name).collect();
    let names = crate::infra::parse_tag_names(&raw)?;
    Ok(AppJson(
        service::page::put_tags(&ctx, &user, id, &names).await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddPageTagRequest {
    name: String,
}

async fn add_page_tag(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
    AppJson(req): AppJson<AddPageTagRequest>,
) -> Result<AppJson<Tag>> {
    let name: TagName = req.name.parse()?;
    Ok(AppJson(
        service::page::add_tag(&ctx, &user, id, &name).await?,
    ))
}

async fn remove_page_tag(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath((page_id, tag_id)): AppPath<(PageId, TagId)>,
) -> Result<StatusCode> {
    service::page::remove_tag(&ctx, &user, page_id, tag_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/page", get(list_pages).post(create_page))
        .route("/page/lookup", get(lookup_page))
        .route(
            "/page/{id}",
            get(get_page).patch(patch_page).delete(delete_page),
        )
        .route("/page/{id}/thumbnail", get(super::capture::get_thumbnail))
        .route(
            "/page/{id}/capture",
            get(list_page_captures)
                .post(super::archive::archive)
                .layer(DefaultBodyLimit::max(ARCHIVE_BODY_LIMIT)),
        )
        .route("/page/{id}/tag", post(add_page_tag).put(put_page_tags))
        .route("/page/{id}/tag/{tag_id}", delete(remove_page_tag))
        .route(
            "/page/{id}/latest/html",
            get(super::capture::redirect_latest_html),
        )
        .route(
            "/page/{id}/latest/html/original",
            get(super::capture::redirect_latest_original_html),
        )
        .route(
            "/page/{id}/latest/screenshot",
            get(super::capture::redirect_latest_screenshot),
        )
}
