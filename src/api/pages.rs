use std::collections::HashMap;

use axum::{extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, FromRow, QueryBuilder, Sqlite, SqlitePool};

use crate::{
    db::helpers::{get_or_create_tag, load_tag, page_tags, replace_url_tags},
    dto::{CaptureSummary, MatchMode, Page, PageQuery, ReplaceTags},
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath, ApiQuery},
    models::TagRow,
    tag::{clean_tag_name, clean_tags},
    url_canon::canonicalize_url,
};

fn clamp_limit(limit: Option<u32>) -> i64 {
    i64::from(limit.unwrap_or(50).clamp(1, 200))
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

const PAGE_SELECT: &str = "SELECT u.id, u.url, u.host,
    COALESCE((SELECT MAX(captured_at) FROM capture WHERE url_id = u.id), 0) AS last_captured_at,
    (SELECT COUNT(*) FROM capture WHERE url_id = u.id) AS capture_count,
    (SELECT title FROM capture WHERE url_id = u.id ORDER BY captured_at DESC, id DESC LIMIT 1) AS title
    FROM url u";

/// Version numbers are computed (1 = oldest) so they renumber if a capture is deleted.
const CAPTURE_SUMMARY_SQL: &str = "SELECT c.id AS id,
    ROW_NUMBER() OVER (ORDER BY c.captured_at, c.id) AS version,
    c.captured_at AS captured_at, c.title AS title, c.status_code AS status_code,
    NULLIF(lower(hex(h.hash)), '') AS content_hash,
    (c.screenshot_content_id IS NOT NULL) AS has_screenshot
    FROM capture c LEFT JOIN content h ON h.id = c.content_id
    WHERE c.url_id = ?";

#[derive(FromRow)]
struct PageRow {
    id: i64,
    url: String,
    host: String,
    last_captured_at: i64, // 0 if no capture exists
    capture_count: i64,
    title: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PageDto {
    id: i64,
    url: String,
    host: String,
    title: Option<String>,
    capture_count: i64,
    last_captured_at: Option<i64>,
    tags: Vec<TagRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PageDetail {
    #[serde(flatten)]
    page: PageDto,
    latest_capture: CaptureSummary,
}

#[derive(FromRow)]
struct UrlTagRow {
    url_id: i64,
    #[sqlx(flatten)]
    tag: TagRow,
}

async fn hydrate_pages(pool: &SqlitePool, rows: Vec<PageRow>) -> Result<Vec<PageDto>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT ut.url_id, t.id, t.name, t.color, t.created_at
         FROM url_tag ut JOIN tag t ON t.id = ut.tag_id
         WHERE ut.url_id IN (",
    );
    let mut sep = qb.separated(", ");
    for r in &rows {
        sep.push_bind(r.id);
    }
    sep.push_unseparated(") ORDER BY t.name COLLATE NOCASE");

    let tag_rows: Vec<UrlTagRow> = qb.build_query_as().fetch_all(pool).await?;
    let mut by_url: HashMap<i64, Vec<TagRow>> = HashMap::new();
    for t in tag_rows {
        by_url.entry(t.url_id).or_default().push(t.tag);
    }

    Ok(rows
        .into_iter()
        .map(|r| PageDto {
            tags: by_url.remove(&r.id).unwrap_or_default(),
            id: r.id,
            url: r.url,
            host: r.host,
            title: r.title,
            capture_count: r.capture_count,
            last_captured_at: (r.capture_count > 0).then_some(r.last_captured_at),
        })
        .collect())
}

async fn load_page(pool: &SqlitePool, id: i64) -> Result<PageDto> {
    let row: PageRow = sqlx::query_as(AssertSqlSafe(format!("{PAGE_SELECT} WHERE u.id = ?")))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .or_not_found("page")?;
    Ok(hydrate_pages(pool, vec![row]).await?.remove(0))
}

async fn page_detail(pool: &SqlitePool, id: i64) -> Result<PageDetail> {
    let page = load_page(pool, id).await?;
    let latest_capture = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT * FROM ({CAPTURE_SUMMARY_SQL}) ORDER BY version DESC LIMIT 1"
    )))
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(PageDetail {
        page,
        latest_capture,
    })
}

async fn ensure_page(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM url WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .or_not_found("page")
        .map(|_| ())
}

// ── GET /api/pages ──────────────────────────────────────────────────────────
// Sorted by most recently archived. In PageQuery, rename the field for the documented param:

#[expect(
    clippy::too_many_lines,
    reason = "builds the dynamic filter, cursor, and pagination in one place"
)]
pub(super) async fn list_pages(
    State(pool): State<SqlitePool>,
    ApiQuery(q): ApiQuery<PageQuery>,
) -> Result<ApiJson<Page<PageDto>>> {
    let limit = clamp_limit(q.limit);
    let tags = clean_tags(&q.tag)?;

    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new(format!("SELECT * FROM ({PAGE_SELECT} WHERE 1=1"));

    if let Some(host) = q.host.as_deref().map(str::trim).filter(|h| !h.is_empty()) {
        qb.push(" AND u.host = ").push_bind(host.to_lowercase());
    }

    if let Some(s) = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let pat = format!("%{}%", escape_like(s));
        qb.push(" AND (u.url LIKE ")
            .push_bind(pat.clone())
            .push(" ESCAPE '\\' OR EXISTS (SELECT 1 FROM capture c WHERE c.url_id = u.id AND c.title LIKE ")
            .push_bind(pat)
            .push(" ESCAPE '\\'))");
    }

    if !tags.is_empty() {
        let joiner = match q.match_mode {
            MatchMode::All => " AND ",
            MatchMode::Any => " OR ",
        };
        qb.push(" AND (");
        for (i, t) in tags.iter().enumerate() {
            if i > 0 {
                qb.push(joiner);
            }
            qb.push(
                "EXISTS (SELECT 1 FROM url_tag ut JOIN tag t ON t.id = ut.tag_id \
                 WHERE ut.url_id = u.id AND t.name = ",
            )
            .push_bind(t.clone())
            .push(")");
        }
        qb.push(")");
    }

    qb.push(") WHERE 1=1");

    if let Some(c) = q.cursor.as_deref() {
        let bad = || AppError::BadRequest("invalid cursor".into());
        let (ts, id) = c.split_once(':').ok_or_else(bad)?;
        let ts: i64 = ts.parse().map_err(|_| bad())?;
        let id: i64 = id.parse().map_err(|_| bad())?;
        qb.push(" AND (last_captured_at, id) < (")
            .push_bind(ts)
            .push(", ")
            .push_bind(id)
            .push(")");
    }

    qb.push(" ORDER BY last_captured_at DESC, id DESC LIMIT ")
        .push_bind(limit.saturating_add(1));

    let mut rows: Vec<PageRow> = qb.build_query_as().fetch_all(&pool).await?;
    let has_more = i64::try_from(rows.len()).map_or(true, |len| len > limit);
    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    let next_cursor = has_more
        .then(|| {
            rows.last()
                .map(|r| format!("{}:{}", r.last_captured_at, r.id))
        })
        .flatten();

    Ok(ApiJson(Page {
        items: hydrate_pages(&pool, rows).await?,
        next_cursor,
    }))
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
    Ok(ApiJson(page_detail(&pool, id).await?))
}

// ── GET /api/pages/{id} ─────────────────────────────────────────────────────

pub(super) async fn get_page(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<PageDetail>> {
    Ok(ApiJson(page_detail(&pool, id).await?))
}

// ── DELETE /api/pages/{id} ──────────────────────────────────────────────────
// Orphaned content rows are left for POST /api/maintenance/gc.

pub(super) async fn delete_page(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<StatusCode> {
    let mut tx = pool.begin().await?;

    sqlx::query_scalar::<_, i64>("SELECT id FROM url WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .or_not_found("page")?;

    // Other pages' captures that redirected to this URL keep existing, minus the link.
    sqlx::query("UPDATE capture SET redirect_url_id = NULL WHERE redirect_url_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // capture_asset rows cascade from capture; the ref_count triggers fire too.
    sqlx::query("DELETE FROM capture WHERE url_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM url WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?; // url_tag cascades

    tx.commit().await?;
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
    ensure_page(&pool, id).await?;
    let limit = clamp_limit(q.limit);

    let mut items: Vec<CaptureSummary> = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT * FROM ({CAPTURE_SUMMARY_SQL}) WHERE version < ? ORDER BY version DESC LIMIT ?"
    )))
    .bind(id)
    .bind(q.cursor.unwrap_or(i64::MAX))
    .bind(limit.saturating_add(1))
    .fetch_all(&pool)
    .await?;

    let has_more = i64::try_from(items.len()).map_or(true, |len| len > limit);
    items.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    let next_cursor = has_more
        .then(|| items.last().map(|c| c.version.to_string()))
        .flatten();
    Ok(ApiJson(Page { items, next_cursor }))
}

// ── Tags on a page ──────────────────────────────────────────────────────────

// PUT /api/pages/{id}/tags   { "tags": [...] }  replaces the full set
pub(super) async fn put_page_tags(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(req): ApiJson<ReplaceTags>,
) -> Result<ApiJson<Vec<TagRow>>> {
    // TODO: also process color
    let names = clean_tags(
        req.tags
            .into_iter()
            .map(|v| v.name)
            .collect::<Vec<_>>()
            .as_slice(),
    )?;
    ensure_page(&pool, id).await?;

    let mut tx = pool.begin().await?;
    replace_url_tags(&mut tx, id, &names).await?;
    let tags = page_tags(&mut tx, id).await?;
    tx.commit().await?;
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
    ensure_page(&pool, id).await?;

    let mut tx = pool.begin().await?;
    let tag_id = get_or_create_tag(&mut tx, &name).await?;
    sqlx::query("INSERT OR IGNORE INTO url_tag (url_id, tag_id) VALUES (?, ?)")
        .bind(id)
        .bind(tag_id)
        .execute(&mut *tx)
        .await?;
    let tag = load_tag(&mut tx, tag_id).await?.or_not_found("tag")?;
    tx.commit().await?;
    Ok(ApiJson(tag))
}

// DELETE /api/pages/{id}/tags/{tag_id}
pub(super) async fn remove_page_tag(
    State(pool): State<SqlitePool>,
    ApiPath((id, tag_id)): ApiPath<(i64, i64)>,
) -> Result<StatusCode> {
    let n = sqlx::query("DELETE FROM url_tag WHERE url_id = ? AND tag_id = ?")
        .bind(id)
        .bind(tag_id)
        .execute(&pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound("tag on page"));
    }
    Ok(StatusCode::NO_CONTENT)
}
