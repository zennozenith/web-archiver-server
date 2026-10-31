use axum::{extract::State, http::StatusCode};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    api::tag::{ReplaceTags, clean_tag_name},
    db::{
        captures::CaptureSummary,
        pages::{
            PageDetail, PageDto, QueryFilter, ensure_page, list_pages_dynamic_filter, page_detail,
        },
        tags::TagRow,
    },
    dto::{Cursor, Page, PageQuery},
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath, ApiQuery},
    tag,
    url_canon::canonicalize_url,
};

// todo: Limit struct
// todo: Cursor struct and how they work

fn clamp_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(50).clamp(1, 200)
}

// ── GET /api/pages ──────────────────────────────────────────────────────────
// Sorted by most recently archived. In PageQuery, rename the field for the documented param:
pub(super) async fn list_pages(
    State(pool): State<SqlitePool>,
    ApiQuery(q): ApiQuery<PageQuery>,
) -> Result<ApiJson<Page<PageDto>>> {
    let limit = clamp_limit(q.limit);
    let tag_names = tag::clean_names(&q.tag)?;

    let cursor = match q.cursor.as_deref() {
        Some(c) => {
            let bad = || AppError::BadRequest("invalid cursor".into());
            let (ts, id) = c.split_once(':').ok_or_else(bad)?;
            let ts: i64 = ts.parse().map_err(|_| bad())?;
            let id: i64 = id.parse().map_err(|_| bad())?;
            Some(Cursor { ts, id })
        }
        None => None,
    };

    let page_dto = list_pages_dynamic_filter(
        &pool,
        QueryFilter {
            host: q.host,
            q: q.q,
            tag_names,
            match_mode: q.match_mode,
            limit,
            cursor,
        },
    )
    .await?;

    Ok(ApiJson(page_dto))
}

// ── GET /api/pages/lookup?url=... ───────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct LookupQuery {
    url: String,
}

pub(super) async fn lookup_page(
    State(pool): State<SqlitePool>,
    ApiQuery(q): ApiQuery<LookupQuery>,
) -> Result<ApiJson<PageDetail>> {
    let canon = canonicalize_url(&q.url)?;
    let id: i64 = sqlx::query_scalar("SELECT id FROM url WHERE url = ?")
        .bind(&canon.url)
        .fetch_optional(&pool)
        .await?
        .or_not_found("page")?;

    Ok(ApiJson(page_detail(&pool, id).await?.or_not_found("page")?))
}

// ── GET /api/pages/{id} ─────────────────────────────────────────────────────

pub(super) async fn get_page(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<PageDetail>> {
    Ok(ApiJson(page_detail(&pool, id).await?.or_not_found("page")?))
}

// ── DELETE /api/pages/{id} ──────────────────────────────────────────────────
// Orphaned content rows are left for POST /api/maintenance/gc.

pub(super) async fn delete_page(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<StatusCode> {
    let found = crate::db::pages::delete_page(&pool, id).await?;

    if !found {
        return Err(AppError::NotFound("page"));
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── GET /api/pages/{id}/captures ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct CapturesQuery {
    limit: Option<u32>,
    cursor: Option<i64>, // last version seen
}

pub(super) async fn list_captures(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
    ApiQuery(q): ApiQuery<CapturesQuery>,
) -> Result<ApiJson<Page<CaptureSummary>>> {
    let found = ensure_page(&pool, id).await?;

    if !found {
        return Err(AppError::NotFound("page"));
    }

    let limit = clamp_limit(q.limit);

    let page = CaptureSummary::list(&pool, id, q.cursor, limit).await?;

    Ok(ApiJson(page))
}

// ── Tags on a page ──────────────────────────────────────────────────────────

// PUT /api/pages/{id}/tags   { "tags": [...] }  replaces the full set
pub(super) async fn put_page_tags(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(req): ApiJson<ReplaceTags>,
) -> Result<ApiJson<Vec<TagRow>>> {
    // TODO: also process color
    let names = tag::clean_names(
        req.tags
            .into_iter()
            .map(|v| v.name)
            .collect::<Vec<_>>()
            .as_slice(),
    )?;
    ensure_page(&pool, id).await?;

    let tags = crate::db::pages::put_page_tags(&pool, id, names).await?;
    Ok(ApiJson(tags))
}

#[derive(Debug, Deserialize)]
pub(super) struct AddTag {
    name: String,
}

// POST /api/pages/{id}/tags   { "name": "..." }  get-or-create + attach
pub(super) async fn add_page_tag(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(req): ApiJson<AddTag>,
) -> Result<ApiJson<TagRow>> {
    let name = clean_tag_name(&req.name)?;
    let tag = crate::db::pages::add_page_tag(&pool, id, name)
        .await?
        .or_not_found("tag")?;

    Ok(ApiJson(tag))
}

// DELETE /api/pages/{id}/tags/{tag_id}
pub(super) async fn remove_page_tag(
    State(pool): State<SqlitePool>,
    ApiPath((id, tag_id)): ApiPath<(i64, i64)>,
) -> Result<StatusCode> {
    let n = crate::db::pages::remove_page_tag(&pool, id, tag_id).await?;
    if n == 0 {
        return Err(AppError::NotFound("tag on page"));
    }
    Ok(StatusCode::NO_CONTENT)
}
