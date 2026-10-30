//! Queries backing the server-rendered web UI.
//!
//! The JSON API has its own handlers in `api::pages`; these helpers exist so the
//! index page can load everything it needs (pages, tags, captures) without an
//! internal HTTP round trip.

use std::collections::HashMap;

use sqlx::{AssertSqlSafe, FromRow, QueryBuilder, Sqlite, SqlitePool};

use crate::models::TagRow;

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

#[derive(Debug, FromRow)]
pub(crate) struct CaptureRow {
    pub id: i64,
    pub url_id: i64,
    pub captured_at: i64,
    pub title: Option<String>,
    pub status_code: Option<i64>,
    pub has_html: bool,
    pub has_screenshot: bool,
}

#[derive(Debug, FromRow)]
pub(crate) struct TagCount {
    pub id: i64,
    pub name: String,
    pub pages: i64,
    pub pending: i64,
}

#[derive(FromRow)]
struct UrlTagRow {
    url_id: i64,
    #[sqlx(flatten)]
    tag: TagRow,
}

/// Newest captures first. The FULL list is loaded so the UI can show versions.
pub(crate) async fn list_pages(pool: &SqlitePool, limit: u32) -> Result<Vec<PageRow>, sqlx::Error> {
    let limit = i64::from(limit.clamp(1, 200));
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
) -> Result<HashMap<i64, Vec<CaptureRow>>, sqlx::Error> {
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

    let rows: Vec<CaptureRow> = qb.build_query_as().fetch_all(pool).await?;
    let mut by_page: HashMap<i64, Vec<CaptureRow>> = HashMap::new();
    for row in rows {
        by_page.entry(row.url_id).or_default().push(row);
    }
    Ok(by_page)
}

pub(crate) async fn list_tag_counts(pool: &SqlitePool) -> Result<Vec<TagCount>, sqlx::Error> {
    sqlx::query_as(
        "SELECT t.id, t.name,
                (SELECT COUNT(*) FROM url_tag ut WHERE ut.tag_id = t.id)     AS pages,
                (SELECT COUNT(*) FROM pending_tag pt WHERE pt.tag_id = t.id) AS pending
         FROM tag t
         ORDER BY pages DESC, t.name COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await
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

pub(crate) async fn page_exists(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM url WHERE id = ?)")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Remove a page, its captures and tag links. Orphaned blobs are left for GC.
pub(crate) async fn delete_page(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
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

/// Insert a tag. Returns `None` when the name already exists.
pub(crate) async fn create_tag(
    pool: &SqlitePool,
    name: &str,
    color: Option<&str>,
) -> Result<Option<TagRow>, sqlx::Error> {
    sqlx::query_as(
        "INSERT INTO tag (name, color) VALUES (?, ?)
         ON CONFLICT(name) DO NOTHING
         RETURNING id, name, color, created_at",
    )
    .bind(name)
    .bind(color)
    .fetch_optional(pool)
    .await
}

pub(crate) async fn rename_tag(
    pool: &SqlitePool,
    id: i64,
    name: &str,
) -> Result<Option<TagRow>, sqlx::Error> {
    sqlx::query_as("UPDATE tag SET name = ? WHERE id = ? RETURNING id, name, color, created_at")
        .bind(name)
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub(crate) async fn tag_name_exists(
    pool: &SqlitePool,
    name: &str,
    except_id: Option<i64>,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM tag WHERE name = ? AND id IS NOT ?")
        .bind(name)
        .bind(except_id)
        .fetch_optional(pool)
        .await
}

pub(crate) async fn delete_tag(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let n = sqlx::query("DELETE FROM tag WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(n > 0)
}
