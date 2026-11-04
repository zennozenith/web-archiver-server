//! Queries for pages (canonicalized URLs), their tags, and their capture summaries.
//!
//! Serves both the JSON API and the server-rendered index, which loads pages,
//! tags and captures directly instead of making an internal HTTP round trip.

use std::{collections::HashMap, fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection, SqliteExecutor, SqlitePool};

use crate::{
    blob_store::BlobStore,
    db::{
        capture::{CaptureOverview, CaptureSummary},
        collection::{CollectionId, non_empty_parent},
        content::ContentId,
        helper::{CursorPage, FieldPatch, InvalidCursor, group_by, id_type},
        store::{insert_blob, prepare_blob},
        tag::Tag,
    },
    error::Result,
    helper::{NonEmptyString, escape_like},
    image::Image,
    limit::Limit,
    tag::{TagId, TagName, comma_separated_tags},
    url_canon::CanonicalUrl,
};

id_type! {
    /// Primary key of a page.
    pub(crate) struct PageId;
}

// ───────────────────────────────────────────────────────────────────────────
// Select
// ───────────────────────────────────────────────────────────────────────────

/// A page with its capture statistics. Aggregates are computed in SQL.
macro_rules! page_record_select {
    () => {
        "SELECT p.id, p.url, p.host,
                COALESCE((SELECT MAX(captured_at) FROM capture WHERE page_id = p.id), 0) AS last_captured_at,
                (SELECT COUNT(*) FROM capture WHERE page_id = p.id) AS capture_count,
                COALESCE(
                    NULLIF(TRIM(p.display_title), ''),
                    (SELECT c.title FROM capture c
                     WHERE c.page_id = p.id
                     ORDER BY c.captured_at DESC, c.id DESC LIMIT 1),
                    p.url
                ) AS display_title,
                p.description, p.collection_id,
                col.name AS collection_name, col.emoji AS collection_emoji,
                p.pinned, p.thumbnail_content_id, p.created_at,
                (SELECT COUNT(*) FROM capture c WHERE c.page_id = p.id) AS capture_count,
                (SELECT MAX(c.captured_at) FROM capture c WHERE c.page_id = p.id) AS last_captured_at,
                EXISTS(SELECT 1 FROM capture c
                       WHERE c.page_id = p.id AND c.content_id IS NOT NULL) AS has_html,
                EXISTS(SELECT 1 FROM capture c
                       WHERE c.page_id = p.id AND c.screenshot_content_id IS NOT NULL) AS has_screenshot
         FROM page p
         LEFT JOIN collection col ON col.id = p.collection_id"
    };
}

// ───────────────────────────────────────────────────────────────────────────
// Models
// ───────────────────────────────────────────────────────────────────────────

#[expect(
    clippy::struct_excessive_bools,
    reason = "pin state and capture-format flags are independent booleans"
)]
#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageRecord {
    pub id: PageId,
    pub url: String,
    pub host: String,

    /// `0` when the page has no captures (keeps keyset comparisons NULL-free).
    pub last_captured_at: i64,
    pub capture_count: i64,

    /// User-editable title when present, otherwise the latest capture title,
    /// otherwise the URL.
    pub display_title: String,
    pub description: String,
    pub collection_id: Option<CollectionId>,
    pub collection_name: Option<String>,
    pub collection_emoji: Option<String>,
    pub pinned: bool,
    pub thumbnail_content_id: Option<ContentId>,
    pub created_at: i64,
    pub has_html: bool,
    pub has_screenshot: bool,
}

/// A page row plus the aggregates the UI needs to render a card.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageItem {
    #[serde(flatten)]
    pub record: PageRecord,
    pub tags: Vec<Tag>,
}

/// A page together with its tags. This is the API representation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageDetail {
    #[serde(flatten)]
    pub page: PageItem,
    /// `None` when the page has no captures yet.
    pub latest_capture: Option<CaptureSummary>,
}

#[derive(Debug, FromRow)]
pub(crate) struct HighlightRecord {
    pub text: String,
    pub color: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct PageStats {
    pub pages: i64,
    pub collections: i64,
    pub tags: i64,
    pub pinned: i64,
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
#[serde(rename_all = "camelCase")]
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

pub(crate) async fn exists<'e>(
    executor: impl SqliteExecutor<'e>,
    id: PageId,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM page WHERE id = ?)")
        .bind(id)
        .fetch_one(executor)
        .await
}

// ───────────────────────────────────────────────────────────────────────────
// Reads
// ───────────────────────────────────────────────────────────────────────────

impl PageRecord {
    pub(crate) async fn load_from_url<'e>(
        executor: impl SqliteExecutor<'e>,
        url: &CanonicalUrl,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(concat!(page_record_select!(), " WHERE p.url = ?"))
            .bind(&url.url)
            .fetch_optional(executor)
            .await
    }

    pub(crate) async fn load<'e>(
        executor: impl SqliteExecutor<'e>,
        id: PageId,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(concat!(page_record_select!(), " WHERE p.id = ?"))
            .bind(id)
            .fetch_optional(executor)
            .await
    }

    // /// Most recently captured pages first.
    // pub(crate) async fn list_recent(
    //     pool: &SqlitePool,
    //     limit: Limit,
    // ) -> Result<Vec<Self>, sqlx::Error> {
    //     sqlx::query_as(concat!(
    //         "SELECT * FROM (",
    //         page_record_select!(),
    //         ") ORDER BY last_captured_at DESC, id DESC LIMIT ?"
    //     ))
    //     .bind(i64::from(limit.get()))
    //     .fetch_all(pool)
    //     .await
    // }
}

impl PageItem {
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

    /// Attaches tags to many records using one extra query.
    pub(crate) async fn hydrate<'e>(
        executor: impl SqliteExecutor<'e>,
        records: Vec<PageRecord>,
    ) -> Result<Vec<Self>> {
        let ids: Vec<PageId> = records.iter().map(|record| record.id).collect();
        let mut tags: HashMap<PageId, Vec<Tag>> = tags_by_page(executor, &ids).await?;

        Ok(records
            .into_iter()
            .map(|record| {
                let tags = tags.remove(&record.id).unwrap_or_default();
                Self { record, tags }
            })
            .collect())
    }

    /// Returns the requested page plus whether another page follows it.
    pub(crate) async fn list(
        pool: &SqlitePool,
        filter: &PageFilter,
    ) -> Result<CursorPage<Self, PageCursor>> {
        let limit = i64::from(filter.limit.get());
        // let offset = i64::try_from(filter.offset).unwrap_or(i64::MAX);

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(concat!(
            "SELECT * FROM (",
            page_record_select!(),
            ") WHERE 1 = 1"
        ));
        filter.push_conditions(&mut qb);

        if let Some(cursor) = filter.cursor {
            qb.push(" AND (last_captured_at, id) < (")
                .push_bind(cursor.captured_at)
                .push(", ")
                .push_bind(cursor.id)
                .push(")");
        }

        match filter.sort {
            PageSort::Newest => {
                qb.push(" ORDER BY created_at DESC, id DESC");
            }
            PageSort::Oldest => {
                qb.push(" ORDER BY created_at ASC, id ASC");
            }
            PageSort::Name => {
                qb.push(" ORDER BY display_title COLLATE NOCASE ASC, id DESC");
            }
        }

        // Fetch one extra row to detect whether another page exists.
        qb.push(" LIMIT ").push_bind(limit.saturating_add(1));
        // .push(" OFFSET ")
        // .push_bind(offset);

        let records: Vec<PageRecord> = qb.build_query_as().fetch_all(pool).await?;
        let keep = usize::try_from(limit).unwrap_or(usize::MAX);

        let page = CursorPage::from_overfetched(records, keep, PageCursor::after);
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
        let latest_capture = CaptureSummary::latest(pool, page.record.id).await?;
        Ok(Some(Self {
            page,
            latest_capture,
        }))
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Search filter
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PageSort {
    #[default]
    Newest,
    Oldest,
    Name,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PageField {
    #[default]
    Name,
    Url,
    Text,
    Tags,
}

/// Filters for the link listing and search pages.
#[derive(Debug)]
pub(crate) struct PageFilter {
    pub text: Option<String>,
    pub field: PageField,
    pub sort: PageSort,
    pub collection_id: Option<CollectionId>,
    pub tag_id: Option<TagId>,
    pub pinned_only: bool,
    pub limit: Limit,
    // pub offset: usize,
    pub cursor: Option<PageCursor>,
}

impl Default for PageFilter {
    fn default() -> Self {
        Self {
            text: None,
            field: PageField::Name,
            sort: PageSort::Newest,
            collection_id: None,
            tag_id: None,
            pinned_only: false,
            limit: Limit::DEFAULT,
            cursor: None,
        }
    }
}

impl PageFilter {
    fn push_conditions(&self, qb: &mut QueryBuilder<Sqlite>) {
        if let Some(collection_id) = self.collection_id {
            qb.push(" AND collection_id = ").push_bind(collection_id);
        }

        if let Some(tag_id) = self.tag_id {
            qb.push(
                " AND EXISTS (SELECT 1 FROM page_tag pt
                  WHERE pt.page_id = id AND pt.tag_id = ",
            )
            .push_bind(tag_id)
            .push(")");
        }

        if self.pinned_only {
            qb.push(" AND pinned = 1");
        }

        if let Some(text) = self
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            let pattern = format!("%{}%", escape_like(text));
            match self.field {
                PageField::Name => {
                    qb.push(" AND display_title LIKE ")
                        .push_bind(pattern)
                        .push(" ESCAPE '\\'");
                }
                PageField::Url => {
                    qb.push(" AND url LIKE ")
                        .push_bind(pattern)
                        .push(" ESCAPE '\\'");
                }
                PageField::Text => {
                    qb.push(" AND description LIKE ")
                        .push_bind(pattern)
                        .push(" ESCAPE '\\'");
                }
                PageField::Tags => {
                    qb.push(
                        " AND EXISTS (SELECT 1 FROM page_tag pt
                          JOIN tag t ON t.id = pt.tag_id
                          WHERE pt.page_id = id AND t.name LIKE ",
                    )
                    .push_bind(pattern)
                    .push(" ESCAPE '\\')");
                }
            }
        }
    }
}

impl HighlightRecord {
    pub(crate) async fn list(pool: &SqlitePool, page_id: PageId) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, text, color, created_at FROM highlight
             WHERE page_id = ? ORDER BY created_at, id",
        )
        .bind(page_id)
        .fetch_all(pool)
        .await
    }

    pub(crate) async fn add(
        pool: &SqlitePool,
        page_id: PageId,
        text: &str,
        color: &str,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as(
            "INSERT INTO highlight (page_id, text, color) VALUES (?, ?, ?)
             RETURNING id, text, color, created_at",
        )
        .bind(page_id)
        .bind(text)
        .bind(color)
        .fetch_one(pool)
        .await
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Mutations
// ───────────────────────────────────────────────────────────────────────────

pub(crate) struct CreatePage {
    pub url: CanonicalUrl,
    pub display_title: NonEmptyString,
    pub description: String,
    pub collection_id: Option<CollectionId>,
    pub pinned: bool,
    pub tags: Vec<TagName>,
    pub thumbnail: Option<Image>,
}

impl CreatePage {
    pub(crate) async fn execute(self, pool: &SqlitePool, store: &BlobStore) -> Result<PageItem> {
        let mut tx = pool.begin().await?;

        let thumbnail = match self.thumbnail {
            Some(image) => {
                let format = image.format();
                let data = image.into_data();
                Some(prepare_blob(pool, store, data, format.mime_type()).await?)
            }
            None => None,
        };

        let thumbnail_id = match &thumbnail {
            Some(b) => Some(insert_blob(&mut tx, b).await?),
            None => None,
        };

        // No-op DO UPDATE so RETURNING yields the existing row's id.
        let page_id: PageId = sqlx::query_scalar(
            "INSERT INTO page (url, scheme, host, path, query,
                display_title, description, collection_id, pinned, thumbnail_content_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(url) DO UPDATE SET url = url
         RETURNING id",
        )
        .bind(self.url.url)
        .bind(self.url.scheme.as_str())
        .bind(self.url.host)
        .bind(self.url.path)
        .bind(self.url.query)
        .bind(self.display_title)
        .bind(self.description)
        .bind(self.collection_id)
        .bind(i32::from(self.pinned))
        .bind(thumbnail_id)
        .fetch_one(&mut *tx)
        .await?;

        replace_tags(&mut tx, page_id, &self.tags).await?;

        tx.commit().await?;

        PageItem::load(pool, page_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound.into())
    }
}

/// PATCH semantics: `value: null` clears, an absent field leaves it unchanged.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePage {
    #[serde(default)]
    pub display_title: FieldPatch<String>,
    #[serde(default)]
    pub description: FieldPatch<String>,
    #[serde(default)]
    pub collection_id: FieldPatch<String>, // i64 string
    #[serde(default)]
    pub pinned: FieldPatch<bool>,
    #[serde(default)]
    pub tags: FieldPatch<String>,
}

impl UpdatePage {
    pub(crate) async fn apply(self, pool: &SqlitePool, id: PageId) -> Result<bool> {
        let Some(current) = PageItem::load(pool, id).await? else {
            return Ok(false);
        };

        let tags = match self.tags {
            FieldPatch::Unchanged => Some(current.tags.into_iter().map(|v| v.name).collect()),
            FieldPatch::Clear => None,
            FieldPatch::Set(t) => Some(comma_separated_tags(Some(&t))?),
        };

        let display_title = self.display_title.apply(Some(current.record.display_title));
        let description = self.description.apply(Some(current.record.description));
        let collection_id = match self.collection_id {
            FieldPatch::Unchanged => current.record.collection_id,
            FieldPatch::Clear => None,
            FieldPatch::Set(s) => CollectionId::parse_or(Some(&s)),
        };

        let pinned = self.pinned.apply(Some(current.record.pinned));

        let collection_id = non_empty_parent(pool, collection_id, None).await?;
        let mut tx = pool.begin().await?;

        sqlx::query(
            "UPDATE page
             SET display_title = COALESCE(NULLIF(TRIM(?), ''), display_title),
                 description = ?,
                 collection_id = ?,
                 pinned = ?
             WHERE id = ?",
        )
        .bind(&display_title)
        .bind(&description)
        .bind(collection_id)
        .bind(pinned)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        if let Some(tags) = tags {
            replace_tags(&mut tx, id, &tags).await?;
        }

        tx.commit().await?;
        Ok(true)
    }
}

impl PageId {
    /// Removes a page and its captures; returns `false` if the page did not exist.
    ///
    /// `page_tag` rows cascade. Orphaned blobs are left for GC.
    pub(crate) async fn delete(self, pool: &SqlitePool) -> Result<bool> {
        let mut tx = pool.begin().await?;

        // Captures of other pages that redirected here keep existing, minus the link.
        sqlx::query("UPDATE capture SET redirect_page_id = NULL WHERE redirect_page_id = ?")
            .bind(self.0)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM capture WHERE page_id = ?")
            .bind(self.0)
            .execute(&mut *tx)
            .await?;
        let deleted = sqlx::query("DELETE FROM page WHERE id = ?")
            .bind(self.0)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;

        tx.commit().await?;
        Ok(deleted)
    }
}

/// Flips the pinned flag. Returns the new state, or `None` if the link is gone.
pub(crate) async fn toggle_pinned<'e>(
    executor: impl SqliteExecutor<'e>,
    id: PageId,
) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar(
        "UPDATE page SET pinned = CASE pinned WHEN 0 THEN 1 ELSE 0 END
         WHERE id = ? RETURNING pinned",
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

pub(crate) async fn stats(pool: &SqlitePool) -> Result<PageStats, sqlx::Error> {
    sqlx::query_as(
        "SELECT
           (SELECT COUNT(*) FROM page) AS pages,
           (SELECT COUNT(*) FROM collection) AS collections,
           (SELECT COUNT(*) FROM tag) AS tags,
           (SELECT COUNT(*) FROM page WHERE pinned = 1) AS pinned",
    )
    .fetch_one(pool)
    .await
}

// ───────────────────────────────────────────────────────────────────────────
// Others
// ───────────────────────────────────────────────────────────────────────────

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

// ───────────────────────────────────────────────────────────────────────────
// Page tags
// ───────────────────────────────────────────────────────────────────────────

pub(crate) async fn tags_of<'e>(
    executor: impl SqliteExecutor<'e>,
    page_id: PageId,
) -> Result<Vec<Tag>, sqlx::Error> {
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
