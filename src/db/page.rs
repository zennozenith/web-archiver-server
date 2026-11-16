//! Queries for pages (canonicalized URLs), their tags, and their capture summaries.
//!
//! Serves both the JSON API and the server-rendered index, which loads pages,
//! tags and captures directly instead of making an internal HTTP round trip.

use std::{collections::HashMap, fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection, SqliteExecutor, SqlitePool};

use crate::{
    db::{
        blob::{insert_blob, prepare_blob},
        capture::CaptureSummary,
        collection::CollectionId,
        content::ContentId,
        cursor::{CursorPage, InvalidCursor},
        group::group_by,
        patch::FieldPatch,
        tag::Tag,
    },
    domain::{TagId, TagName, comma_separated_tags, ids::id_type},
    error::{AppError, Resource, Result},
    infra::{BlobFiles, Image},
    util::{
        CanonicalUrl, NonEmptyString, like_contains_pattern, limit::Limit,
        url_canon::canonicalize_url,
    },
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
        "SELECT p.id, p.owner_user_id, p.url, p.host,
                COALESCE((SELECT MAX(c.captured_at) FROM captures c WHERE c.page_id = p.id), 0) AS last_captured_at,
                (SELECT COUNT(*) FROM captures c WHERE c.page_id = p.id) AS capture_count,
                COALESCE(
                    NULLIF(TRIM(p.display_title), ''),
                    (SELECT c.title FROM captures c
                     WHERE c.page_id = p.id
                     ORDER BY c.captured_at DESC, c.id DESC LIMIT 1),
                    p.url
                ) AS display_title,
                p.description, p.collection_id,
                col.name AS collection_name, col.emoji AS collection_emoji,
                p.pinned, p.thumbnail_content_id, p.created_at,
                EXISTS(SELECT 1 FROM captures c
                       WHERE c.page_id = p.id AND c.content_id IS NOT NULL) AS has_html,
                EXISTS(SELECT 1 FROM captures c
                       WHERE c.page_id = p.id AND c.screenshot_content_id IS NOT NULL) AS has_screenshot
         FROM pages p
         LEFT JOIN collections col ON col.id = p.collection_id"
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
pub(crate) struct PageItem {
    #[serde(flatten)]
    pub record: PageRecord,
    pub tags: Vec<Tag>,
}

/// A page together with its tags. This is the API representation.
#[derive(Debug, Serialize)]
pub(crate) struct PageDetail {
    #[serde(flatten)]
    pub page: PageItem,
    /// `None` when the page has no captures yet.
    pub latest_capture: Option<CaptureSummary>,
}

#[derive(Debug, FromRow)]
pub(crate) struct HighlightRecord {
    pub id: i64,
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

/// Keyset position for pages ordered by `(created_at, id)`.
/// Serialized as `"<created_at>:<id>"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub(crate) struct PageCursor {
    pub created_at: i64,
    pub id: PageId,
}

impl PageCursor {
    const fn after(record: &PageRecord) -> Self {
        Self {
            created_at: record.created_at,
            id: record.id,
        }
    }
}

impl fmt::Display for PageCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.created_at, self.id)
    }
}

impl FromStr for PageCursor {
    type Err = InvalidCursor;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (created_at, id) = s.split_once(':').ok_or(InvalidCursor)?;
        Ok(Self {
            created_at: created_at.parse().map_err(|_| InvalidCursor)?,
            id: PageId::from(id.parse::<i64>().map_err(|_| InvalidCursor)?),
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
    owner: i64,
    id: PageId,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pages WHERE owner_user_id = ? AND id = ?)")
        .bind(owner)
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
        owner: i64,
        url: &CanonicalUrl,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(concat!(
            page_record_select!(),
            " WHERE p.owner_user_id = ? AND p.url = ?"
        ))
        .bind(owner)
        .bind(&url.url)
        .fetch_optional(executor)
        .await
    }

    pub(crate) async fn load<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        id: PageId,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(concat!(
            page_record_select!(),
            " WHERE p.owner_user_id = ? AND p.id = ?"
        ))
        .bind(owner)
        .bind(id)
        .fetch_optional(executor)
        .await
    }
}

impl PageItem {
    pub(crate) async fn load(pool: &SqlitePool, owner: i64, id: PageId) -> Result<Option<Self>> {
        let Some(record) = PageRecord::load(pool, owner, id).await? else {
            return Ok(None);
        };
        Ok(Self::hydrate(pool, owner, vec![record]).await?.pop())
    }

    pub(crate) async fn load_by_url(
        pool: &SqlitePool,
        owner: i64,
        url: &CanonicalUrl,
    ) -> Result<Option<Self>> {
        let Some(record) = PageRecord::load_from_url(pool, owner, url).await? else {
            return Ok(None);
        };
        Ok(Self::hydrate(pool, owner, vec![record]).await?.pop())
    }

    /// Attaches tags to many records using one extra query.
    pub(crate) async fn hydrate<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        records: Vec<PageRecord>,
    ) -> Result<Vec<Self>> {
        let ids: Vec<PageId> = records.iter().map(|record| record.id).collect();
        let mut tags: HashMap<PageId, Vec<Tag>> = tags_by_page(executor, owner, &ids).await?;

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

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(concat!(
            "SELECT * FROM (",
            page_record_select!(),
            ") p WHERE 1 = 1"
        ));
        qb.push(" AND p.owner_user_id = ")
            .push_bind(filter.owner_user_id);
        filter.push_conditions(&mut qb);

        // The cursor only carries the created-at ordering key, so alphabetical
        // listings are served as a single page and ignore it.
        if let Some(cursor) = filter.cursor.filter(|_| filter.sort != PageSort::Name) {
            qb.push(" AND (p.created_at, p.id) < (")
                .push_bind(cursor.created_at)
                .push(", ")
                .push_bind(cursor.id)
                .push(")");
        }

        match filter.sort {
            PageSort::Newest => {
                qb.push(" ORDER BY p.created_at DESC, p.id DESC");
            }
            PageSort::Oldest => {
                qb.push(" ORDER BY p.created_at ASC, p.id ASC");
            }
            PageSort::Name => {
                qb.push(" ORDER BY p.display_title COLLATE NOCASE ASC, p.id DESC");
            }
        }

        // Fetch one extra row to detect whether another page exists.
        qb.push(" LIMIT ").push_bind(limit.saturating_add(1));

        let records: Vec<PageRecord> = qb.build_query_as().fetch_all(pool).await?;
        let keep = usize::try_from(limit).unwrap_or(usize::MAX);

        let page = CursorPage::from_overfetched(records, keep, PageCursor::after);
        let items = Self::hydrate(pool, filter.owner_user_id, page.items).await?;

        Ok(CursorPage {
            items,
            next_cursor: if filter.sort == PageSort::Name {
                None
            } else {
                page.next_cursor
            },
        })
    }
}

impl PageDetail {
    pub(crate) async fn load(pool: &SqlitePool, owner: i64, id: PageId) -> Result<Option<Self>> {
        let Some(page) = PageItem::load(pool, owner, id).await? else {
            return Ok(None);
        };
        let latest_capture = CaptureSummary::latest(pool, owner, id).await?;
        Ok(Some(Self {
            page,
            latest_capture,
        }))
    }

    pub(crate) async fn load_by_url(
        pool: &SqlitePool,
        owner: i64,
        url: &CanonicalUrl,
    ) -> Result<Option<Self>> {
        let Some(page) = PageItem::load_by_url(pool, owner, url).await? else {
            return Ok(None);
        };
        let latest_capture = CaptureSummary::latest(pool, owner, page.record.id).await?;
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
pub(crate) enum PageSort {
    #[default]
    Newest,
    Oldest,
    Name,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
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
    pub owner_user_id: i64,
    pub text: Option<String>,
    pub field: PageField,
    pub sort: PageSort,
    pub collection_id: Option<CollectionId>,
    /// Only links that are not filed in any collection.
    pub unorganized_only: bool,
    pub tag_id: Option<TagId>,
    pub pinned_only: bool,
    pub limit: Limit,
    pub cursor: Option<PageCursor>,
}

impl Default for PageFilter {
    fn default() -> Self {
        Self {
            owner_user_id: 0,
            text: None,
            field: PageField::Name,
            sort: PageSort::Newest,
            collection_id: None,
            unorganized_only: false,
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
            qb.push(" AND p.collection_id = ").push_bind(collection_id);
        }

        if self.unorganized_only {
            qb.push(" AND p.collection_id IS NULL");
        }

        if let Some(tag_id) = self.tag_id {
            qb.push(
                " AND EXISTS (SELECT 1 FROM page_tags pt
                  WHERE pt.page_id = p.id AND pt.tag_id = ",
            )
            .push_bind(tag_id)
            .push(")");
        }

        if self.pinned_only {
            qb.push(" AND p.pinned = 1");
        }

        if let Some(text) = self
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            let pattern = like_contains_pattern(text);
            match self.field {
                PageField::Name => {
                    qb.push(" AND p.display_title LIKE ")
                        .push_bind(pattern)
                        .push(" ESCAPE '\\'");
                }
                PageField::Url => {
                    qb.push(" AND p.url LIKE ")
                        .push_bind(pattern)
                        .push(" ESCAPE '\\'");
                }
                PageField::Text => {
                    qb.push(" AND p.description LIKE ")
                        .push_bind(pattern)
                        .push(" ESCAPE '\\'");
                }
                PageField::Tags => {
                    qb.push(
                        " AND EXISTS (SELECT 1 FROM page_tags pt
                          JOIN tags t ON t.id = pt.tag_id
                          WHERE pt.page_id = p.id AND t.name LIKE ",
                    )
                    .push_bind(pattern)
                    .push(" ESCAPE '\\')");
                }
            }
        }
    }
}

impl HighlightRecord {
    pub(crate) async fn list(
        pool: &SqlitePool,
        owner: i64,
        page_id: PageId,
    ) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as(
            "SELECT h.id, h.text, h.color, h.created_at
             FROM highlights h JOIN pages p ON p.id = h.page_id
             WHERE p.owner_user_id = ? AND h.page_id = ?
             ORDER BY h.created_at, h.id",
        )
        .bind(owner)
        .bind(page_id)
        .fetch_all(pool)
        .await
    }

    pub(crate) async fn add(
        pool: &SqlitePool,
        owner: i64,
        page_id: PageId,
        text: &str,
        color: &str,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as(
            "INSERT INTO highlights (page_id, text, color)
             SELECT id, ?, ? FROM pages WHERE owner_user_id = ? AND id = ?
             RETURNING id, text, color, created_at",
        )
        .bind(text)
        .bind(color)
        .bind(owner)
        .bind(page_id)
        .fetch_one(pool)
        .await
    }

    pub(crate) async fn update(
        pool: &SqlitePool,
        owner: i64,
        id: i64,
        text: Option<&str>,
        color: Option<&str>,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as(
            "UPDATE highlights
             SET text = COALESCE(?, text), color = COALESCE(?, color)
             WHERE id = ?
               AND page_id IN (SELECT id FROM pages WHERE owner_user_id = ?)
             RETURNING id, text, color, created_at",
        )
        .bind(text)
        .bind(color)
        .bind(id)
        .bind(owner)
        .fetch_optional(pool)
        .await
    }

    pub(crate) async fn delete(
        pool: &SqlitePool,
        owner: i64,
        id: i64,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM highlights
             WHERE id = ?
               AND page_id IN (SELECT id FROM pages WHERE owner_user_id = ?)",
        )
        .bind(id)
        .bind(owner)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
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
    pub(crate) async fn execute(
        self,
        pool: &SqlitePool,
        owner: i64,
        store: &BlobFiles,
    ) -> Result<PageItem> {
        // Hash, compress and write the blob before taking the write connection:
        // a one-connection pool would otherwise deadlock on `prepare_blob`'s
        // dedup query.
        let thumbnail = match self.thumbnail {
            Some(image) => {
                let format = image.format();
                let data = image.into_data();
                Some(prepare_blob(pool, store, data, format.mime_type()).await?)
            }
            None => None,
        };

        let mut tx = pool.begin().await?;

        let thumbnail_id = match &thumbnail {
            Some(b) => Some(insert_blob(&mut tx, b).await?),
            None => None,
        };

        // No-op DO UPDATE so RETURNING yields the existing row's id.
        let page_id: PageId = sqlx::query_scalar(
            "INSERT INTO pages (owner_user_id, url, scheme, host, path, query,
                display_title, description, collection_id, pinned, thumbnail_content_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(owner_user_id, url) DO UPDATE SET url = url
         RETURNING id",
        )
        .bind(owner)
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

        replace_tags(&mut tx, owner, page_id, &self.tags).await?;

        tx.commit().await?;

        PageItem::load(pool, owner, page_id)
            .await?
            .ok_or_else(|| AppError::Conflict("page was removed concurrently, please retry".into()))
    }
}

/// PATCH semantics: `value: null` clears, an absent field leaves it unchanged.
#[derive(Debug, Default, Deserialize)]
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
    pub(crate) async fn apply(self, pool: &SqlitePool, owner: i64, id: PageId) -> Result<bool> {
        let Some(current) = PageItem::load(pool, owner, id).await? else {
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
            FieldPatch::Set(s) => CollectionId::parse_opt(Some(s.as_str())),
        };

        let pinned = self.pinned.apply(Some(current.record.pinned));

        let mut tx = pool.begin().await?;

        sqlx::query(
            "UPDATE pages
             SET display_title = COALESCE(NULLIF(TRIM(?), ''), display_title),
                 description = ?,
                 collection_id = ?,
                 pinned = ?
             WHERE owner_user_id = ? AND id = ?",
        )
        .bind(&display_title)
        .bind(&description)
        .bind(collection_id)
        .bind(pinned)
        .bind(owner)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        if let Some(tags) = tags {
            replace_tags(&mut tx, owner, id, &tags).await?;
        }

        tx.commit().await?;
        Ok(true)
    }
}

impl PageId {
    pub(crate) const fn get(self) -> i64 {
        self.0
    }

    /// Replaces the page's thumbnail. `None` removes the custom thumbnail.
    /// Returns `false` when the page does not exist.
    pub(crate) async fn set_thumbnail(
        self,
        pool: &SqlitePool,
        owner: i64,
        store: &BlobFiles,
        thumbnail: Option<Image>,
    ) -> Result<bool> {
        // See `CreatePage::execute`: prepare outside the transaction so a
        // one-connection pool cannot deadlock.
        let blob = match thumbnail {
            Some(image) => {
                let format = image.format();
                let data = image.into_data();
                Some(prepare_blob(pool, store, data, format.mime_type()).await?)
            }
            None => None,
        };

        let mut tx = pool.begin().await?;

        let thumbnail_id = match &blob {
            Some(blob) => Some(insert_blob(&mut tx, blob).await?),
            None => None,
        };

        let updated = sqlx::query(
            "UPDATE pages SET thumbnail_content_id = ?
             WHERE owner_user_id = ? AND id = ?",
        )
        .bind(thumbnail_id)
        .bind(owner)
        .bind(self)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;

        tx.commit().await?;
        Ok(updated)
    }

    /// Removes a page and its captures; returns `false` if the page did not exist.
    ///
    /// `page_tags` rows cascade. Orphaned blobs are left for GC.
    pub(crate) async fn delete(self, pool: &SqlitePool, owner: i64) -> Result<bool> {
        let mut tx = pool.begin().await?;

        // Captures of other pages that redirected here keep existing, minus the link.
        sqlx::query(
            "UPDATE captures SET redirect_page_id = NULL
             WHERE redirect_page_id = ?",
        )
        .bind(self.0)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM captures WHERE page_id = ?")
            .bind(self.0)
            .execute(&mut *tx)
            .await?;
        let deleted = sqlx::query("DELETE FROM pages WHERE owner_user_id = ? AND id = ?")
            .bind(owner)
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
    owner: i64,
    id: PageId,
) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar(
        "UPDATE pages SET pinned = CASE pinned WHEN 0 THEN 1 ELSE 0 END
         WHERE owner_user_id = ? AND id = ? RETURNING pinned",
    )
    .bind(owner)
    .bind(id)
    .fetch_optional(executor)
    .await
}

pub(crate) async fn stats(pool: &SqlitePool, owner: i64) -> Result<PageStats, sqlx::Error> {
    sqlx::query_as(
        "SELECT
           (SELECT COUNT(*) FROM pages WHERE owner_user_id = ?1) AS pages,
           (SELECT COUNT(*) FROM collections WHERE owner_user_id = ?1) AS collections,
           (SELECT COUNT(*) FROM tags WHERE owner_user_id = ?1) AS tags,
           (SELECT COUNT(*) FROM pages WHERE owner_user_id = ?1 AND pinned = 1) AS pinned",
    )
    .bind(owner)
    .fetch_one(pool)
    .await
}

// ───────────────────────────────────────────────────────────────────────────
// Page tags
// ───────────────────────────────────────────────────────────────────────────

pub(crate) async fn tags_of<'e>(
    executor: impl SqliteExecutor<'e>,
    owner: i64,
    page_id: PageId,
) -> Result<Vec<Tag>, sqlx::Error> {
    sqlx::query_as(
        "SELECT t.id, t.name, t.color, t.created_at
         FROM page_tags pt
         JOIN tags t ON t.id = pt.tag_id
         JOIN pages p ON p.id = pt.page_id
         WHERE p.owner_user_id = ? AND pt.page_id = ?
         ORDER BY t.name COLLATE NOCASE",
    )
    .bind(owner)
    .bind(page_id)
    .fetch_all(executor)
    .await
}

/// Tags for many pages in one query, grouped by page id.
pub(crate) async fn tags_by_page<'e>(
    executor: impl SqliteExecutor<'e>,
    owner: i64,
    ids: &[PageId],
) -> Result<HashMap<PageId, Vec<Tag>>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT pt.page_id, t.id, t.name, t.color, t.created_at
         FROM page_tags pt
         JOIN tags t ON t.id = pt.tag_id
         JOIN pages p ON p.id = pt.page_id
         WHERE p.owner_user_id = ",
    );
    qb.push_bind(owner).push(" AND pt.page_id IN (");
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
    owner: i64,
    page_id: PageId,
    names: &[TagName],
) -> Result<()> {
    for name in names {
        let tag_id = Tag::get_or_create_id(&mut *conn, owner, name).await?;
        sqlx::query("INSERT OR IGNORE INTO page_tags (page_id, tag_id) VALUES (?, ?)")
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
    owner: i64,
    page_id: PageId,
    names: &[TagName],
) -> Result<()> {
    sqlx::query("DELETE FROM page_tags WHERE page_id = ?")
        .bind(page_id)
        .execute(&mut *conn)
        .await?;
    add_tags(conn, owner, page_id, names).await
}

/// Replaces the page's tags and returns the new set; `None` if the page does not exist.
pub(crate) async fn put_tags(
    pool: &SqlitePool,
    owner: i64,
    page_id: PageId,
    names: &[TagName],
) -> Result<Option<Vec<Tag>>> {
    let mut tx = pool.begin().await?;

    if !exists(&mut *tx, owner, page_id).await? {
        return Ok(None);
    }
    replace_tags(&mut tx, owner, page_id, names).await?;
    let tags = tags_of(&mut *tx, owner, page_id).await?;

    tx.commit().await?;
    Ok(Some(tags))
}

/// Adds one tag (creating it if needed); `None` if the page does not exist.
pub(crate) async fn add_tag(
    pool: &SqlitePool,
    owner: i64,
    page_id: PageId,
    name: &TagName,
) -> Result<Option<Tag>> {
    let mut tx = pool.begin().await?;

    if !exists(&mut *tx, owner, page_id).await? {
        return Ok(None);
    }

    let tag_id = Tag::get_or_create_id(&mut *tx, owner, name).await?;
    sqlx::query("INSERT OR IGNORE INTO page_tags (page_id, tag_id) VALUES (?, ?)")
        .bind(page_id)
        .bind(tag_id)
        .execute(&mut *tx)
        .await?;
    let tag = Tag::load(&mut *tx, owner, tag_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Tag))?;

    tx.commit().await?;
    Ok(Some(tag))
}

/// Returns `true` if the tag was attached to the page.
pub(crate) async fn remove_tag(
    pool: &SqlitePool,
    owner: i64,
    page_id: PageId,
    tag_id: TagId,
) -> Result<bool> {
    let result = sqlx::query(
        "DELETE FROM page_tags
         WHERE page_id = ? AND tag_id = ?
           AND EXISTS (SELECT 1 FROM pages WHERE id = ? AND owner_user_id = ?)",
    )
    .bind(page_id)
    .bind(tag_id)
    .bind(page_id)
    .bind(owner)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Canonicalizes a raw URL for the create flow.
pub(crate) fn parse_url(raw: &str) -> Result<CanonicalUrl> {
    canonicalize_url(raw)
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;
    use crate::infra::{Image, ImageFormat};

    #[tokio::test]
    async fn create_page_with_thumbnail_on_one_connection() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        crate::db::migrate(&pool).await.expect("migrations");

        let owner: i64 = sqlx::query_scalar(
            "INSERT INTO users (email, status) VALUES ('thumb@example.com', 'active') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert user");

        let dir = std::env::temp_dir().join(format!(
            "web-archiver-page-test-{}-{}",
            std::process::id(),
            crate::util::time::now_ms()
        ));
        let store = BlobFiles::open(&dir, 1024).await.expect("blob store");

        let image = Image::new(
            ImageFormat::Png,
            b"\x89PNG\r\n\x1a\nthumbnail-bytes".to_vec(),
        )
        .expect("png");
        let page = CreatePage {
            url: canonicalize_url("https://example.com/thumb").expect("url"),
            display_title: NonEmptyString::trimmed("Thumbnail").expect("title"),
            description: String::new(),
            collection_id: None,
            pinned: false,
            tags: Vec::new(),
            thumbnail: Some(image),
        };

        let created = page
            .execute(&pool, owner, &store)
            .await
            .expect("create page");
        assert!(created.record.thumbnail_content_id.is_some());

        let reloaded = PageItem::load(&pool, owner, created.record.id)
            .await
            .expect("load page")
            .expect("page exists");
        assert!(reloaded.record.thumbnail_content_id.is_some());

        drop(tokio::fs::remove_dir_all(dir).await);
    }
}
