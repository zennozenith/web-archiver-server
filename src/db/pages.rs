//! Queries backing the server-rendered web UI.
//!
//! The JSON API has its own handlers in `api::pages`; these helpers exist so the
//! index page can load everything it needs (pages, tags, captures) without an
//! internal HTTP round trip.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::{AssertSqlSafe, FromRow, QueryBuilder, Result, Sqlite, SqliteConnection, SqlitePool};

use crate::{
    db::{
        captures::{CaptureForPageRow, CaptureSummary},
        tags::{TagRow, get_or_create_tag},
    },
    dto::{Cursor, MatchMode, Page},
    limit::Limit,
    tag::Name,
    url_canon::CanonicalUrl,
};

const PAGE_SELECT: &str = "SELECT u.id, u.url, u.host,
    COALESCE((SELECT MAX(captured_at) FROM capture WHERE url_id = u.id), 0) AS last_captured_at,
    (SELECT COUNT(*) FROM capture WHERE url_id = u.id) AS capture_count,
    (SELECT title FROM capture WHERE url_id = u.id ORDER BY captured_at DESC, id DESC LIMIT 1) AS title
    FROM url u";

#[derive(Debug, FromRow)]
pub(crate) struct PageRow {
    pub id: i64,
    pub url: String,
    pub host: String,
    pub last_captured_at: i64,
    pub capture_count: i64,
    pub title: Option<String>,
}

#[derive(FromRow)]
struct UrlTagRow {
    url_id: i64,
    #[sqlx(flatten)]
    tag: TagRow,
}

pub(crate) async fn get_or_create_url(
    conn: &mut SqliteConnection,
    canon: &CanonicalUrl,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO url (url, scheme, host, path, query) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(url) DO UPDATE SET url = url
         RETURNING id",
    )
    .bind(&canon.url)
    .bind(canon.scheme.as_str())
    .bind(&canon.host)
    .bind(&canon.path)
    .bind(&canon.query)
    .fetch_one(&mut *conn)
    .await
}

pub(crate) async fn replace_page_tags(
    conn: &mut SqliteConnection,
    page_id: i64,
    names: &[Name],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM url_tag WHERE url_id = ?")
        .bind(page_id)
        .execute(&mut *conn)
        .await?;
    add_page_tags(conn, page_id, names).await
}

/// Union tags into a page (never removes existing ones).
pub(crate) async fn add_page_tags(
    conn: &mut SqliteConnection,
    url_id: i64,
    names: &[Name],
) -> Result<(), sqlx::Error> {
    for name in names {
        let tag_id = get_or_create_tag(&mut *conn, name).await?;
        sqlx::query("INSERT OR IGNORE INTO url_tag (url_id, tag_id) VALUES (?, ?)")
            .bind(url_id)
            .bind(tag_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Newest captures first. The FULL list is loaded so the UI can show versions.
pub(crate) async fn list_pages(
    pool: &SqlitePool,
    limit: Limit,
) -> Result<Vec<PageRow>, sqlx::Error> {
    let limit = i64::from(limit.get());
    sqlx::query_as(AssertSqlSafe(format!(
        "SELECT * FROM ({PAGE_SELECT}) ORDER BY last_captured_at DESC, id DESC LIMIT ?"
    )))
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Tags for many pages in one query, grouped by page id.
pub(crate) async fn tags_for_pages(
    pool: &SqlitePool,
    ids: &[i64],
) -> Result<HashMap<i64, Vec<TagRow>>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT ut.url_id, t.id, t.name, t.color, t.created_at
         FROM url_tag ut JOIN tag t ON t.id = ut.tag_id
         WHERE ut.url_id IN (",
    );
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(*id);
    }
    sep.push_unseparated(") ORDER BY t.name COLLATE NOCASE");

    let rows: Vec<UrlTagRow> = qb.build_query_as().fetch_all(pool).await?;
    let mut by_page: HashMap<i64, Vec<TagRow>> = HashMap::new();
    for row in rows {
        by_page.entry(row.url_id).or_default().push(row.tag);
    }
    Ok(by_page)
}

/// Captures for many pages in one query, newest first per page.
pub(crate) async fn captures_for_pages(
    pool: &SqlitePool,
    ids: &[i64],
) -> Result<HashMap<i64, Vec<CaptureForPageRow>>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT c.id, c.url_id, c.captured_at, c.title, c.status_code,
                (c.content_id IS NOT NULL) AS has_html,
                (c.screenshot_content_id IS NOT NULL) AS has_screenshot
         FROM capture c WHERE c.url_id IN (",
    );
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(*id);
    }
    sep.push_unseparated(") ORDER BY c.url_id, c.captured_at DESC, c.id DESC");

    let rows: Vec<CaptureForPageRow> = qb.build_query_as().fetch_all(pool).await?;
    let mut by_page: HashMap<i64, Vec<CaptureForPageRow>> = HashMap::new();
    for row in rows {
        by_page.entry(row.url_id).or_default().push(row);
    }
    Ok(by_page)
}

pub(crate) async fn page_tags(pool: &SqlitePool, page_id: i64) -> Result<Vec<TagRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT t.id, t.name, t.color, t.created_at
         FROM url_tag ut JOIN tag t ON t.id = ut.tag_id
         WHERE ut.url_id = ? ORDER BY t.name COLLATE NOCASE",
    )
    .bind(page_id)
    .fetch_all(pool)
    .await
}

pub(crate) async fn page_tags_with_conn(
    conn: &mut SqliteConnection,
    page_id: i64,
) -> Result<Vec<TagRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT t.id, t.name, t.color, t.created_at
         FROM url_tag ut JOIN tag t ON t.id = ut.tag_id
         WHERE ut.url_id = ? ORDER BY t.name COLLATE NOCASE",
    )
    .bind(page_id)
    .fetch_all(&mut *conn)
    .await
}

pub(crate) async fn page_exists(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM url WHERE id = ?)")
        .bind(id)
        .fetch_one(pool)
        .await
}

// ====================================================

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageDto {
    id: i64,
    url: String,
    host: String,
    title: Option<String>,
    capture_count: i64,
    last_captured_at: Option<i64>,
    tags: Vec<TagRow>,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageDetail {
    #[serde(flatten)]
    page: PageDto,
    latest_capture: CaptureSummary,
}

async fn load_page(pool: &SqlitePool, id: i64) -> Result<Option<PageDto>> {
    let Some(row) = sqlx::query_as(AssertSqlSafe(format!("{PAGE_SELECT} WHERE u.id = ?")))
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(hydrate_pages(pool, vec![row]).await?.remove(0)))
}

pub(crate) async fn page_detail(pool: &SqlitePool, id: i64) -> Result<Option<PageDetail>> {
    let Some(page) = load_page(pool, id).await? else {
        return Ok(None);
    };

    let latest_capture = CaptureSummary::latest(pool, id).await?;
    Ok(Some(PageDetail {
        page,
        latest_capture,
    }))
}

pub(crate) async fn ensure_page(pool: &SqlitePool, id: i64) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT id FROM url WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .is_some(),
    )
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(crate) struct QueryFilter {
    pub host: Option<String>,
    pub q: Option<String>,
    pub tag_names: Vec<Name>,
    pub match_mode: MatchMode,
    pub limit: Limit,
    pub cursor: Option<Cursor>,
}

/// Sorted by most recently archived. In `PageQuery`, rename the field for the documented param:
#[expect(
    clippy::too_many_lines,
    reason = "builds the dynamic filter, cursor, and pagination in one place"
)]
pub(crate) async fn list_pages_dynamic_filter(
    pool: &SqlitePool,
    filter: QueryFilter,
) -> Result<Page<PageDto>> {
    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new(format!("SELECT * FROM ({PAGE_SELECT} WHERE 1=1"));

    if let Some(host) = filter
        .host
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty())
    {
        qb.push(" AND u.host = ").push_bind(host.to_lowercase());
    }

    if let Some(s) = filter.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let pat = format!("%{}%", escape_like(s));
        qb.push(" AND (u.url LIKE ")
            .push_bind(pat.clone())
            .push(" ESCAPE '\\' OR EXISTS (SELECT 1 FROM capture c WHERE c.url_id = u.id AND c.title LIKE ")
            .push_bind(pat)
            .push(" ESCAPE '\\'))");
    }

    if !filter.tag_names.is_empty() {
        let joiner = match filter.match_mode {
            MatchMode::All => " AND ",
            MatchMode::Any => " OR ",
        };
        qb.push(" AND (");
        for (i, t) in filter.tag_names.iter().enumerate() {
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

    if let Some(Cursor { ts, id }) = filter.cursor {
        qb.push(" AND (last_captured_at, id) < (")
            .push_bind(ts)
            .push(", ")
            .push_bind(id)
            .push(")");
    }

    qb.push(" ORDER BY last_captured_at DESC, id DESC LIMIT ")
        .push_bind(filter.limit.get().saturating_add(1));

    let mut rows: Vec<PageRow> = qb.build_query_as().fetch_all(pool).await?;
    let has_more = i64::try_from(rows.len()).map_or(true, |len| len > filter.limit.get().into());
    rows.truncate(usize::from(filter.limit));
    let next_cursor = has_more
        .then(|| {
            rows.last()
                .map(|r| format!("{}:{}", r.last_captured_at, r.id))
        })
        .flatten();

    Ok(Page {
        items: hydrate_pages(pool, rows).await?,
        next_cursor,
    })
}

/// Remove a page, its captures and tag links. Orphaned blobs are left for GC.
pub(crate) async fn delete_page(pool: &SqlitePool, id: i64) -> Result<bool> {
    let mut tx = pool.begin().await?;

    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM url WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_none() {
        return Ok(false);
    }

    // Other pages' captures that redirected here keep existing, minus the link.
    sqlx::query("UPDATE capture SET redirect_url_id = NULL WHERE redirect_url_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM capture WHERE url_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM url WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(true)
}

pub(crate) async fn put_page_tags(
    pool: &SqlitePool,
    page_id: i64,
    names: Vec<Name>,
) -> Result<Vec<TagRow>> {
    ensure_page(pool, page_id).await?;

    let mut tx = pool.begin().await?;
    replace_page_tags(&mut tx, page_id, &names).await?;
    let tags = page_tags_with_conn(&mut tx, page_id).await?;
    tx.commit().await?;

    Ok(tags)
}

pub(crate) async fn add_page_tag(
    pool: &SqlitePool,
    page_id: i64,
    name: Name,
) -> Result<Option<TagRow>> {
    ensure_page(pool, page_id).await?;

    let mut tx = pool.begin().await?;
    let tag_id = get_or_create_tag(&mut tx, &name).await?;
    sqlx::query("INSERT OR IGNORE INTO url_tag (url_id, tag_id) VALUES (?, ?)")
        .bind(page_id)
        .bind(tag_id)
        .execute(&mut *tx)
        .await?;
    let tag = TagRow::load(&mut tx, tag_id).await?;
    tx.commit().await?;

    Ok(tag)
}

pub(crate) async fn remove_page_tag(pool: &SqlitePool, page_id: i64, tag_id: i64) -> Result<u64> {
    let n = sqlx::query("DELETE FROM url_tag WHERE url_id = ? AND tag_id = ?")
        .bind(page_id)
        .bind(tag_id)
        .execute(pool)
        .await?
        .rows_affected();

    Ok(n)
}
