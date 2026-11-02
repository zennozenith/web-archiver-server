//! Links are archived pages with bookmark metadata: title, notes, collection,
//! tags, pin state and highlights.

use std::collections::HashMap;

use sqlx::{FromRow, QueryBuilder, Result, Sqlite, SqliteExecutor, SqlitePool};

use crate::{
    db::{
        collection::CollectionId,
        page::{self, PageId},
        tag::Tag,
    },
    limit::Limit,
    tag::{TagId, TagName},
    url_canon::CanonicalUrl,
};

// ───────────────────────────────────────────────────────────────────────────
// Models
// ───────────────────────────────────────────────────────────────────────────

/// A link row plus the aggregates the UI needs to render a card.
#[derive(Debug, FromRow)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "pin state and capture-format flags are independent booleans"
)]
pub(crate) struct LinkRecord {
    pub id: PageId,
    pub url: String,
    pub host: String,
    /// User-editable title when present, otherwise the latest capture title,
    /// otherwise the URL.
    pub display_title: String,
    pub description: String,
    pub collection_id: Option<CollectionId>,
    pub collection_name: Option<String>,
    pub collection_emoji: Option<String>,
    pub pinned: bool,
    pub thumbnail_url: Option<String>,
    pub created_at: i64,
    pub has_html: bool,
    pub has_screenshot: bool,
}

/// A link together with its tags.
#[derive(Debug)]
pub(crate) struct LinkItem {
    pub record: LinkRecord,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum LinkSort {
    #[default]
    Newest,
    Oldest,
    Name,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum LinkField {
    #[default]
    Name,
    Url,
    Text,
    Tags,
}

/// Filters for the link listing and search pages.
#[derive(Debug)]
pub(crate) struct LinkFilter {
    pub text: Option<String>,
    pub field: LinkField,
    pub sort: LinkSort,
    pub collection_id: Option<CollectionId>,
    pub tag_id: Option<TagId>,
    pub pinned_only: bool,
    pub limit: Limit,
    pub offset: usize,
}

impl Default for LinkFilter {
    fn default() -> Self {
        Self {
            text: None,
            field: LinkField::Name,
            sort: LinkSort::Newest,
            collection_id: None,
            tag_id: None,
            pinned_only: false,
            limit: Limit::DEFAULT,
            offset: 0,
        }
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct HighlightRecord {
    pub text: String,
    pub color: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct LinkStats {
    pub links: i64,
    pub collections: i64,
    pub tags: i64,
    pub pinned: i64,
}

// ───────────────────────────────────────────────────────────────────────────
// Select
// ───────────────────────────────────────────────────────────────────────────

macro_rules! link_select {
    () => {
        "SELECT p.id, p.url, p.host, p.title,
                COALESCE(
                    NULLIF(TRIM(p.title), ''),
                    (SELECT c.title FROM capture c
                     WHERE c.page_id = p.id
                     ORDER BY c.captured_at DESC, c.id DESC LIMIT 1),
                    p.url
                ) AS display_title,
                p.description, p.collection_id,
                col.name AS collection_name, col.emoji AS collection_emoji,
                p.pinned, p.thumbnail_url, p.created_at,
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
// Reads
// ───────────────────────────────────────────────────────────────────────────

impl LinkItem {
    pub(crate) async fn load(pool: &SqlitePool, id: PageId) -> Result<Option<Self>> {
        let Some(record) =
            sqlx::query_as::<_, LinkRecord>(concat!(link_select!(), " WHERE p.id = ?"))
                .bind(id)
                .fetch_optional(pool)
                .await?
        else {
            return Ok(None);
        };
        Ok(Self::hydrate(pool, vec![record]).await?.pop())
    }

    /// Attaches tags to many records using one extra query.
    pub(crate) async fn hydrate<'e>(
        executor: impl SqliteExecutor<'e>,
        records: Vec<LinkRecord>,
    ) -> Result<Vec<Self>> {
        let ids: Vec<PageId> = records.iter().map(|record| record.id).collect();
        let mut tags: HashMap<PageId, Vec<Tag>> = page::tags_by_page(executor, &ids).await?;

        Ok(records
            .into_iter()
            .map(|record| {
                let tags = tags.remove(&record.id).unwrap_or_default();
                Self { record, tags }
            })
            .collect())
    }

    /// Returns the requested page plus whether another page follows it.
    pub(crate) async fn list(pool: &SqlitePool, filter: &LinkFilter) -> Result<(Vec<Self>, bool)> {
        let limit = i64::from(filter.limit.get());
        let offset = i64::try_from(filter.offset).unwrap_or(i64::MAX);

        let mut qb: QueryBuilder<Sqlite> =
            QueryBuilder::new(concat!("SELECT * FROM (", link_select!(), ") WHERE 1 = 1"));
        filter.push_conditions(&mut qb);

        match filter.sort {
            LinkSort::Newest => {
                qb.push(" ORDER BY created_at DESC, id DESC");
            }
            LinkSort::Oldest => {
                qb.push(" ORDER BY created_at ASC, id ASC");
            }
            LinkSort::Name => {
                qb.push(" ORDER BY display_title COLLATE NOCASE ASC, id DESC");
            }
        }

        // Fetch one extra row to detect whether another page exists.
        qb.push(" LIMIT ")
            .push_bind(limit.saturating_add(1))
            .push(" OFFSET ")
            .push_bind(offset);

        let mut records: Vec<LinkRecord> = qb.build_query_as().fetch_all(pool).await?;
        let keep = usize::try_from(limit).unwrap_or(usize::MAX);
        let has_more = records.len() > keep;
        records.truncate(keep);

        Ok((Self::hydrate(pool, records).await?, has_more))
    }
}

/// Escapes `%`, `_` and `\` so user text is matched literally by `LIKE ... ESCAPE '\'`.
fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

impl LinkFilter {
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
                LinkField::Name => {
                    qb.push(" AND display_title LIKE ")
                        .push_bind(pattern)
                        .push(" ESCAPE '\\'");
                }
                LinkField::Url => {
                    qb.push(" AND url LIKE ")
                        .push_bind(pattern)
                        .push(" ESCAPE '\\'");
                }
                LinkField::Text => {
                    qb.push(" AND description LIKE ")
                        .push_bind(pattern)
                        .push(" ESCAPE '\\'");
                }
                LinkField::Tags => {
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
    pub(crate) async fn list(pool: &SqlitePool, page_id: PageId) -> Result<Vec<Self>> {
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
    ) -> Result<Self> {
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

/// Creates the page if the URL is new, otherwise updates the existing link.
#[expect(
    clippy::too_many_arguments,
    reason = "link metadata is a wide row; grouping it would only add boilerplate"
)]
pub(crate) async fn create(
    pool: &SqlitePool,
    url: &CanonicalUrl,
    title: Option<&str>,
    description: &str,
    collection_id: Option<CollectionId>,
    tags: &[TagName],
    thumbnail_url: Option<&str>,
) -> Result<LinkItem> {
    let title = title.map(str::trim).filter(|title| !title.is_empty());
    let thumbnail_url = thumbnail_url
        .map(str::trim)
        .filter(|thumbnail| !thumbnail.is_empty());

    let mut tx = pool.begin().await?;
    let page_id = page::get_or_create_id(&mut *tx, url).await?;

    sqlx::query(
        "UPDATE page
         SET title = COALESCE(?, title),
             description = ?,
             collection_id = ?,
             thumbnail_url = COALESCE(?, thumbnail_url)
         WHERE id = ?",
    )
    .bind(title)
    .bind(description)
    .bind(collection_id)
    .bind(thumbnail_url)
    .bind(page_id)
    .execute(&mut *tx)
    .await?;

    page::replace_tags(&mut tx, page_id, tags).await?;

    tx.commit().await?;

    LinkItem::load(pool, page_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

/// Updates editable fields and replaces the link's tag set.
#[expect(
    clippy::too_many_arguments,
    reason = "link metadata is a wide row; grouping it would only add boilerplate"
)]
pub(crate) async fn update(
    pool: &SqlitePool,
    id: PageId,
    title: Option<&str>,
    description: &str,
    collection_id: Option<CollectionId>,
    tags: &[TagName],
) -> Result<bool> {
    let mut tx = pool.begin().await?;

    if !page::exists(&mut *tx, id).await? {
        return Ok(false);
    }

    sqlx::query(
        "UPDATE page
         SET title = COALESCE(NULLIF(TRIM(?), ''), title),
             description = ?,
             collection_id = ?
         WHERE id = ?",
    )
    .bind(title.unwrap_or_default())
    .bind(description)
    .bind(collection_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    page::replace_tags(&mut tx, id, tags).await?;

    tx.commit().await?;
    Ok(true)
}

/// Flips the pinned flag. Returns the new state, or `None` if the link is gone.
pub(crate) async fn toggle_pinned<'e>(
    executor: impl SqliteExecutor<'e>,
    id: PageId,
) -> Result<Option<bool>> {
    sqlx::query_scalar(
        "UPDATE page SET pinned = CASE pinned WHEN 0 THEN 1 ELSE 0 END
         WHERE id = ? RETURNING pinned",
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

pub(crate) async fn delete(pool: &SqlitePool, id: PageId) -> Result<bool> {
    page::delete(pool, id).await
}

pub(crate) async fn stats(pool: &SqlitePool) -> Result<LinkStats> {
    sqlx::query_as(
        "SELECT
           (SELECT COUNT(*) FROM page) AS links,
           (SELECT COUNT(*) FROM collection) AS collections,
           (SELECT COUNT(*) FROM tag) AS tags,
           (SELECT COUNT(*) FROM page WHERE pinned = 1) AS pinned",
    )
    .fetch_one(pool)
    .await
}
