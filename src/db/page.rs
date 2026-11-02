//! Queries for pages (canonicalized URLs), their tags, and their capture summaries.
//!
//! Serves both the JSON API and the server-rendered index, which loads pages,
//! tags and captures directly instead of making an internal HTTP round trip.

use std::{collections::HashMap, fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Result, Sqlite, SqliteConnection, SqliteExecutor, SqlitePool};

use crate::{
    db::{
        capture::{CaptureOverview, CaptureSummary},
        helper::{CursorPage, InvalidCursor, MatchMode, group_by, id_type},
        tag::Tag,
    },
    limit::Limit,
    tag::{TagId, TagName},
    url_canon::CanonicalUrl,
};

id_type! {
    /// Primary key of a page.
    pub(crate) struct PageId;
}

/// A page with its capture statistics. Aggregates are computed in SQL.
macro_rules! page_summary_select {
    () => {
        "SELECT p.id, p.url, p.host,
            COALESCE((SELECT MAX(captured_at) FROM capture WHERE page_id = p.id), 0) AS last_captured_at,
            (SELECT COUNT(*) FROM capture WHERE page_id = p.id) AS capture_count,
            (SELECT title FROM capture WHERE page_id = p.id
              ORDER BY captured_at DESC, id DESC LIMIT 1) AS title
         FROM page p"
    };
}

// ───────────────────────────────────────────────────────────────────────────
// Models
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, FromRow)]
pub(crate) struct PageRecord {
    pub id: PageId,
    pub url: String,
    pub host: String,
    /// `0` when the page has no captures (keeps keyset comparisons NULL-free).
    pub last_captured_at: i64,
    pub capture_count: i64,
    /// Title of the newest capture.
    pub title: Option<String>,
}

/// A page together with its tags. This is the API representation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageItem {
    pub id: PageId,
    pub url: String,
    pub host: String,
    pub title: Option<String>,
    pub capture_count: i64,
    /// `None` when the page has no captures.
    pub last_captured_at: Option<i64>,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageDetail {
    #[serde(flatten)]
    pub page: PageItem,
    /// `None` when the page has no captures yet.
    pub latest_capture: Option<CaptureSummary>,
}

#[derive(FromRow)]
struct PageTagJoin {
    page_id: PageId,
    #[sqlx(flatten)]
    tag: Tag,
}

// ───────────────────────────────────────────────────────────────────────────
// Cursor for the filtered listing
// ───────────────────────────────────────────────────────────────────────────

/// Keyset position for pages ordered by `(last_captured_at, id)` descending.
/// Serialized as `"<captured_at>:<id>"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub(crate) struct PageCursor {
    pub captured_at: i64,
    pub id: PageId,
}

impl PageCursor {
    const fn after(record: &PageRecord) -> Self {
        Self {
            captured_at: record.last_captured_at,
            id: record.id,
        }
    }
}

impl fmt::Display for PageCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.captured_at, self.id)
    }
}

impl FromStr for PageCursor {
    type Err = InvalidCursor;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (captured_at, id) = s.split_once(':').ok_or(InvalidCursor)?;
        Ok(Self {
            captured_at: captured_at.parse().map_err(|_| InvalidCursor)?,
            id: PageId::new(id.parse().map_err(|_| InvalidCursor)?),
        })
    }
}

impl From<PageCursor> for String {
    fn from(cursor: PageCursor) -> Self {
        cursor.to_string()
    }
}

impl TryFrom<String> for PageCursor {
    type Error = InvalidCursor;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Page table operations
// ───────────────────────────────────────────────────────────────────────────

pub(crate) async fn exists<'e>(executor: impl SqliteExecutor<'e>, id: PageId) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM page WHERE id = ?)")
        .bind(id)
        .fetch_one(executor)
        .await
}

pub(crate) async fn get_or_create_id<'e>(
    executor: impl SqliteExecutor<'e>,
    url: &CanonicalUrl,
) -> Result<PageId> {
    // No-op DO UPDATE so RETURNING yields the existing row's id.
    sqlx::query_scalar(
        "INSERT INTO page (url, scheme, host, path, query) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(url) DO UPDATE SET url = url
         RETURNING id",
    )
    .bind(&url.url)
    .bind(url.scheme.as_str())
    .bind(&url.host)
    .bind(&url.path)
    .bind(&url.query)
    .fetch_one(executor)
    .await
}

/// Removes a page and its captures; returns `false` if the page did not exist.
///
/// `page_tag` rows cascade. Orphaned blobs are left for GC.
pub(crate) async fn delete(pool: &SqlitePool, id: PageId) -> Result<bool> {
    let mut tx = pool.begin().await?;

    // Captures of other pages that redirected here keep existing, minus the link.
    sqlx::query("UPDATE capture SET redirect_page_id = NULL WHERE redirect_page_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM capture WHERE page_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let deleted = sqlx::query("DELETE FROM page WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        > 0;

    tx.commit().await?;
    Ok(deleted)
}

// ───────────────────────────────────────────────────────────────────────────
// Page tags
// ───────────────────────────────────────────────────────────────────────────

pub(crate) async fn tags_of<'e>(
    executor: impl SqliteExecutor<'e>,
    page_id: PageId,
) -> Result<Vec<Tag>> {
    sqlx::query_as(
        "SELECT t.id, t.name, t.color, t.created_at
         FROM page_tag pt JOIN tag t ON t.id = pt.tag_id
         WHERE pt.page_id = ? ORDER BY t.name COLLATE NOCASE",
    )
    .bind(page_id)
    .fetch_all(executor)
    .await
}

/// Tags for many pages in one query, grouped by page id.
pub(crate) async fn tags_by_page<'e>(
    executor: impl SqliteExecutor<'e>,
    ids: &[PageId],
) -> Result<HashMap<PageId, Vec<Tag>>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT pt.page_id, t.id, t.name, t.color, t.created_at
         FROM page_tag pt JOIN tag t ON t.id = pt.tag_id
         WHERE pt.page_id IN (",
    );
    let mut list = qb.separated(", ");
    for &id in ids {
        list.push_bind(id);
    }
    list.push_unseparated(") ORDER BY t.name COLLATE NOCASE");

    let joins: Vec<PageTagJoin> = qb.build_query_as().fetch_all(executor).await?;
    Ok(group_by(joins.into_iter().map(|j| (j.page_id, j.tag))))
}

/// Union `names` into the page's tags (never removes existing ones).
pub(crate) async fn add_tags(
    conn: &mut SqliteConnection,
    page_id: PageId,
    names: &[TagName],
) -> Result<()> {
    for name in names {
        let tag_id = Tag::get_or_create_id(&mut *conn, name).await?;
        sqlx::query("INSERT OR IGNORE INTO page_tag (page_id, tag_id) VALUES (?, ?)")
            .bind(page_id)
            .bind(tag_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Replace the page's whole tag set with `names`.
pub(crate) async fn replace_tags(
    conn: &mut SqliteConnection,
    page_id: PageId,
    names: &[TagName],
) -> Result<()> {
    sqlx::query("DELETE FROM page_tag WHERE page_id = ?")
        .bind(page_id)
        .execute(&mut *conn)
        .await?;
    add_tags(conn, page_id, names).await
}

/// Replaces the page's tags and returns the new set; `None` if the page does not exist.
pub(crate) async fn put_tags(
    pool: &SqlitePool,
    page_id: PageId,
    names: &[TagName],
) -> Result<Option<Vec<Tag>>> {
    let mut tx = pool.begin().await?;

    if !exists(&mut *tx, page_id).await? {
        return Ok(None);
    }
    replace_tags(&mut tx, page_id, names).await?;
    let tags = tags_of(&mut *tx, page_id).await?;

    tx.commit().await?;
    Ok(Some(tags))
}

/// Adds one tag (creating it if needed); `None` if the page does not exist.
pub(crate) async fn add_tag(
    pool: &SqlitePool,
    page_id: PageId,
    name: &TagName,
) -> Result<Option<Tag>> {
    let mut tx = pool.begin().await?;

    if !exists(&mut *tx, page_id).await? {
        return Ok(None);
    }

    let tag_id = Tag::get_or_create_id(&mut *tx, name).await?;
    sqlx::query("INSERT OR IGNORE INTO page_tag (page_id, tag_id) VALUES (?, ?)")
        .bind(page_id)
        .bind(tag_id)
        .execute(&mut *tx)
        .await?;
    let tag = Tag::load(&mut *tx, tag_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;

    tx.commit().await?;
    Ok(Some(tag))
}

/// Returns `true` if the tag was attached to the page.
pub(crate) async fn remove_tag(pool: &SqlitePool, page_id: PageId, tag_id: TagId) -> Result<bool> {
    let result = sqlx::query("DELETE FROM page_tag WHERE page_id = ? AND tag_id = ?")
        .bind(page_id)
        .bind(tag_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

// ───────────────────────────────────────────────────────────────────────────
// Reads
// ───────────────────────────────────────────────────────────────────────────

impl PageRecord {
    pub(crate) async fn load_from_url(
        pool: &SqlitePool,
        url: &CanonicalUrl,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(page_summary_select!(), " WHERE p.url = ?"))
            .bind(&url.url)
            .fetch_optional(pool)
            .await
    }

    pub(crate) async fn load(pool: &SqlitePool, id: PageId) -> Result<Option<Self>> {
        sqlx::query_as(concat!(page_summary_select!(), " WHERE p.id = ?"))
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    /// Most recently captured pages first.
    pub(crate) async fn list_recent(pool: &SqlitePool, limit: Limit) -> Result<Vec<Self>> {
        sqlx::query_as(concat!(
            "SELECT * FROM (",
            page_summary_select!(),
            ") ORDER BY last_captured_at DESC, id DESC LIMIT ?"
        ))
        .bind(i64::from(limit.get()))
        .fetch_all(pool)
        .await
    }
}

impl PageItem {
    fn new(record: PageRecord, tags: Vec<Tag>) -> Self {
        Self {
            id: record.id,
            url: record.url,
            host: record.host,
            title: record.title,
            capture_count: record.capture_count,
            last_captured_at: (record.capture_count > 0).then_some(record.last_captured_at),
            tags,
        }
    }

    pub(crate) async fn load(pool: &SqlitePool, id: PageId) -> Result<Option<Self>> {
        let Some(record) = PageRecord::load(pool, id).await? else {
            return Ok(None);
        };
        Ok(Self::hydrate(pool, vec![record]).await?.pop())
    }

    pub(crate) async fn load_by_url(pool: &SqlitePool, url: &CanonicalUrl) -> Result<Option<Self>> {
        let Some(record) = PageRecord::load_from_url(pool, url).await? else {
            return Ok(None);
        };
        Ok(Self::hydrate(pool, vec![record]).await?.pop())
    }

    /// Attaches tags to many records using a single extra query.
    pub(crate) async fn hydrate<'e>(
        executor: impl SqliteExecutor<'e>,
        records: Vec<PageRecord>,
    ) -> Result<Vec<Self>> {
        let ids: Vec<PageId> = records.iter().map(|r| r.id).collect();
        let mut tags = tags_by_page(executor, &ids).await?;

        Ok(records
            .into_iter()
            .map(|record| {
                let tags = tags.remove(&record.id).unwrap_or_default();
                Self::new(record, tags)
            })
            .collect())
    }

    /// Filtered, newest-first keyset search.
    pub(crate) async fn search(
        pool: &SqlitePool,
        filter: &PageFilter,
    ) -> Result<CursorPage<Self, PageCursor>> {
        let limit = i64::from(filter.limit.get());

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(concat!(
            "SELECT * FROM (",
            page_summary_select!(),
            " WHERE 1 = 1"
        ));
        filter.push_conditions(&mut qb);
        qb.push(") WHERE 1 = 1");

        if let Some(cursor) = filter.cursor {
            qb.push(" AND (last_captured_at, id) < (")
                .push_bind(cursor.captured_at)
                .push(", ")
                .push_bind(cursor.id)
                .push(")");
        }
        // Fetch one extra row to detect whether another page exists.
        qb.push(" ORDER BY last_captured_at DESC, id DESC LIMIT ")
            .push_bind(limit.saturating_add(1));

        let records: Vec<PageRecord> = qb.build_query_as().fetch_all(pool).await?;

        let page = CursorPage::from_overfetched(
            records,
            usize::try_from(limit).unwrap_or(usize::MAX),
            PageCursor::after,
        );
        let items = Self::hydrate(pool, page.items).await?;

        Ok(CursorPage {
            items,
            next_cursor: page.next_cursor,
        })
    }
}

impl PageDetail {
    pub(crate) async fn load(pool: &SqlitePool, id: PageId) -> Result<Option<Self>> {
        let Some(page) = PageItem::load(pool, id).await? else {
            return Ok(None);
        };
        let latest_capture = CaptureSummary::latest(pool, id).await?;
        Ok(Some(Self {
            page,
            latest_capture,
        }))
    }

    pub(crate) async fn load_by_url(pool: &SqlitePool, url: &CanonicalUrl) -> Result<Option<Self>> {
        let Some(page) = PageItem::load_by_url(pool, url).await? else {
            return Ok(None);
        };
        let latest_capture = CaptureSummary::latest(pool, page.id).await?;
        Ok(Some(Self {
            page,
            latest_capture,
        }))
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Search filter
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) struct PageFilter {
    /// Exact host match (case-insensitive).
    pub host: Option<String>,
    /// Substring match against the URL or any capture title.
    pub text: Option<String>,
    /// Must be unique (see `parse_names`).
    pub tags: Vec<TagName>,
    pub tag_match: MatchMode,
    pub limit: Limit,
    pub cursor: Option<PageCursor>,
}

/// Escapes `%`, `_` and `\` so user text is matched literally by `LIKE ... ESCAPE '\'`.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

impl PageFilter {
    /// Appends `AND ...` conditions on the `page p` row of [`page_summary_select!`].
    fn push_conditions(&self, qb: &mut QueryBuilder<Sqlite>) {
        if let Some(host) = self
            .host
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
        {
            qb.push(" AND p.host = ").push_bind(host.to_lowercase());
        }

        if let Some(text) = self
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            let pattern = format!("%{}%", escape_like(text));
            qb.push(" AND (p.url LIKE ")
                .push_bind(pattern.clone())
                .push(
                    " ESCAPE '\\' OR EXISTS (SELECT 1 FROM capture c \
                     WHERE c.page_id = p.id AND c.title LIKE ",
                )
                .push_bind(pattern)
                .push(" ESCAPE '\\'))");
        }

        if !self.tags.is_empty() {
            qb.push(
                " AND (SELECT COUNT(DISTINCT t.id) FROM page_tag pt \
                 JOIN tag t ON t.id = pt.tag_id \
                 WHERE pt.page_id = p.id AND t.name IN (",
            );
            let mut names = qb.separated(", ");
            for name in &self.tags {
                names.push_bind(name);
            }
            names.push_unseparated("))");

            match self.tag_match {
                MatchMode::All => {
                    let wanted = i64::try_from(self.tags.len()).unwrap_or(i64::MAX);
                    qb.push(" = ").push_bind(wanted);
                }
                MatchMode::Any => {
                    qb.push(" > 0");
                }
            }
        }
    }
}

#[expect(dead_code, reason = "Might be needed later")]
/// Captures for many pages in one query, newest first per page.
pub(crate) async fn captures_for_pages(
    pool: &SqlitePool,
    ids: &[i64],
) -> Result<HashMap<PageId, Vec<CaptureOverview>>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT c.id, c.page_id, c.captured_at, c.title, c.status_code,
                (c.content_id IS NOT NULL) AS has_html,
                (c.screenshot_content_id IS NOT NULL) AS has_screenshot
         FROM capture c WHERE c.page_id IN (",
    );
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(*id);
    }
    sep.push_unseparated(") ORDER BY c.page_id, c.captured_at DESC, c.id DESC");

    let rows: Vec<CaptureOverview> = qb.build_query_as().fetch_all(pool).await?;
    let mut by_page: HashMap<PageId, Vec<CaptureOverview>> = HashMap::new();
    for row in rows {
        by_page.entry(row.page_id).or_default().push(row);
    }
    Ok(by_page)
}
