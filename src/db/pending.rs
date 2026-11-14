use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Result, Sqlite, SqliteConnection, SqliteExecutor, SqlitePool};

use crate::{
    db::{
        create::CreateOutcome,
        cursor::{Cursor, CursorPage},
        ids::id_type,
        page::PageId,
        tag::Tag,
    },
    infra::TagName,
    util::{limit::Limit, url_canon::CanonicalUrl},
};

/// Column list shared by every query that materializes a [`PendingRow`].
macro_rules! pending_columns {
    () => {
        "id, url, status, attempts, last_error, created_at"
    };
}

// ───────────────────────────────────────────────────────────────────────────
// Types
// ───────────────────────────────────────────────────────────────────────────

id_type! {
    /// Primary key of a pending item.
    pub(crate) struct PendingId;
}

impl PendingId {
    pub(crate) const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, strum::Display)]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub(crate) enum PendingStatus {
    Queued,
    InProgress,
    Failed,
}

/// A row of the `pending_items` table.
#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingRow {
    pub id: PendingId,
    pub url: String,
    pub status: PendingStatus,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
}

/// A pending record together with its tags. This is the API representation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingResponse {
    #[serde(flatten)]
    pub record: PendingRow,
    pub tags: Vec<Tag>,
}

// ───────────────────────────────────────────────────────────────────────────
// Single-record operations
// ───────────────────────────────────────────────────────────────────────────

impl PendingRow {
    pub(crate) async fn load<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        id: PendingId,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "SELECT ",
            pending_columns!(),
            " FROM pending_items WHERE owner_user_id = ? AND id = ?"
        ))
        .bind(owner)
        .bind(id)
        .fetch_optional(executor)
        .await
    }

    pub(crate) async fn exists<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        id: PendingId,
    ) -> Result<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pending_items WHERE owner_user_id = ? AND id = ?)",
        )
        .bind(owner)
        .bind(id)
        .fetch_one(executor)
        .await
    }

    /// Current status, or `None` if the item does not exist.
    pub(crate) async fn status_of<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        id: PendingId,
    ) -> Result<Option<PendingStatus>> {
        sqlx::query_scalar("SELECT status FROM pending_items WHERE owner_user_id = ? AND id = ?")
            .bind(owner)
            .bind(id)
            .fetch_optional(executor)
            .await
    }

    /// Queues `url`, or reports the existing item if the URL is already known.
    ///
    /// Tags are attached only when the item is new.
    /// Shared by the single and bulk create endpoints.
    pub(crate) async fn enqueue(
        conn: &mut SqliteConnection,
        owner: i64,
        url: &CanonicalUrl,
        tags: &[TagName],
    ) -> crate::error::Result<CreateOutcome<PendingId>> {
        let inserted: Option<PendingId> = sqlx::query_scalar(
            "INSERT INTO pending_items (owner_user_id, url) VALUES (?, ?)
             ON CONFLICT(owner_user_id, url) DO NOTHING
             RETURNING id",
        )
        .bind(owner)
        .bind(&url.url)
        .fetch_optional(&mut *conn)
        .await?;

        if let Some(id) = inserted {
            Self::replace_tags(&mut *conn, owner, id, tags).await?;
            return Ok(CreateOutcome::Created(id));
        }

        let id =
            sqlx::query_scalar("SELECT id FROM pending_items WHERE owner_user_id = ? AND url = ?")
                .bind(owner)
                .bind(&url.url)
                .fetch_optional(&mut *conn)
                .await?
                .ok_or_else(|| {
                    crate::error::AppError::Conflict(
                        "pending item was removed concurrently, please retry".into(),
                    )
                })?;
        // The URL was already queued; merge the requested tags into it instead
        // of dropping them.
        Self::merge_tags(&mut *conn, owner, id, tags).await?;
        Ok(CreateOutcome::Existing(id))
    }

    /// Replaces the item's tag set with `names`, creating missing tags.
    pub(crate) async fn replace_tags(
        conn: &mut SqliteConnection,
        owner: i64,
        id: PendingId,
        names: &[TagName],
    ) -> Result<()> {
        sqlx::query("DELETE FROM pending_item_tags WHERE pending_item_id = ?")
            .bind(id)
            .execute(&mut *conn)
            .await?;

        Self::merge_tags(conn, owner, id, names).await
    }

    /// Adds `names` to the item's existing tags, ignoring duplicates.
    pub(crate) async fn merge_tags(
        conn: &mut SqliteConnection,
        owner: i64,
        id: PendingId,
        names: &[TagName],
    ) -> Result<()> {
        for name in names {
            let tag_id = Tag::get_or_create_id(&mut *conn, owner, name).await?;
            sqlx::query(
                "INSERT OR IGNORE INTO pending_item_tags (pending_item_id, tag_id)
                 VALUES (?, ?)",
            )
            .bind(id)
            .bind(tag_id)
            .execute(&mut *conn)
            .await?;
        }
        Ok(())
    }

    /// Atomically takes the oldest queued item and marks it `in_progress`.
    ///
    /// A single `UPDATE ... RETURNING` guarantees two workers never claim the same row.
    /// `owner` scopes the queue to one user; `None` claims across users (admin workers).
    pub(crate) async fn claim_next<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: Option<i64>,
        now: i64,
    ) -> Result<Option<PendingId>> {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "UPDATE pending_items
             SET status = 'in_progress', locked_at = ",
        );
        qb.push_bind(now).push(", attempts = attempts + 1 WHERE id = (SELECT id FROM pending_items WHERE status = 'queued' AND next_attempt_at <= ")
            .push_bind(now);
        if let Some(owner) = owner {
            qb.push(" AND owner_user_id = ").push_bind(owner);
        }
        qb.push(" ORDER BY next_attempt_at, id LIMIT 1) RETURNING id");

        qb.build_query_scalar().fetch_optional(executor).await
    }

    /// Moves a `failed` item back to `queued`. Returns `false` if it was not failed.
    pub(crate) async fn retry<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        id: PendingId,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE pending_items SET status = 'queued', last_error = NULL,
                    next_attempt_at = CAST(round(unixepoch('subsec') * 1000) AS INTEGER)
             WHERE owner_user_id = ? AND id = ? AND status = 'failed'",
        )
        .bind(owner)
        .bind(id)
        .execute(executor)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Marks an `in_progress` item as failed. Returns `false` if it was not in progress.
    pub(crate) async fn mark_failed<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        id: PendingId,
        error: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE pending_items
             SET status = 'failed', last_error = ?, locked_at = NULL
             WHERE owner_user_id = ? AND id = ? AND status = 'in_progress'",
        )
        .bind(error)
        .bind(owner)
        .bind(id)
        .execute(executor)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Deletes an item unless a worker is processing it. Returns `true` if deleted.
    pub(crate) async fn delete<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        id: PendingId,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM pending_items
             WHERE owner_user_id = ? AND id = ? AND status != 'in_progress'",
        )
        .bind(owner)
        .bind(id)
        .execute(executor)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Counts items with the given status.
    pub(crate) async fn count_with_status<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        status: PendingStatus,
    ) -> Result<i64> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM pending_items WHERE owner_user_id = ? AND status = ?",
        )
        .bind(owner)
        .bind(status)
        .fetch_one(executor)
        .await
    }

    /// Deletes every item with `status`; returns how many were removed.
    pub(crate) async fn delete_all_with_status<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        status: PendingStatus,
    ) -> Result<u64> {
        let result =
            sqlx::query("DELETE FROM pending_items WHERE owner_user_id = ? AND status = ?")
                .bind(owner)
                .bind(status)
                .execute(executor)
                .await?;
        Ok(result.rows_affected())
    }

    /// Copies a pending item's tags onto the page and removes it from the queue.
    ///
    /// The pending row must belong to `owner`; a missing row is a no-op.
    pub(crate) async fn complete_into_page(
        conn: &mut SqliteConnection,
        owner: i64,
        pending_id: PendingId,
        page_id: PageId,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO page_tags (page_id, tag_id)
             SELECT ?, pit.tag_id
               FROM pending_item_tags pit
               JOIN pending_items pi ON pi.id = pit.pending_item_id
              WHERE pi.id = ? AND pi.owner_user_id = ?",
        )
        .bind(page_id)
        .bind(pending_id)
        .bind(owner)
        .execute(&mut *conn)
        .await?;

        sqlx::query("DELETE FROM pending_items WHERE id = ? AND owner_user_id = ?")
            .bind(pending_id)
            .bind(owner)
            .execute(&mut *conn)
            .await?;
        Ok(())
    }

    /// Call once at startup: anything left `in_progress` belongs to a dead worker.
    #[expect(
        dead_code,
        reason = "startup recovery hook for deployments with external workers"
    )]
    pub(crate) async fn requeue_stalled<'e>(executor: impl SqliteExecutor<'e>) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE pending_items SET status = 'queued', locked_at = NULL
             WHERE status = 'in_progress'",
        )
        .execute(executor)
        .await?;
        Ok(result.rows_affected())
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Partial update
// ───────────────────────────────────────────────────────────────────────────

/// Fields to change on a pending item; unset fields are left untouched.
#[derive(Debug, Default)]
pub(crate) struct PendingUpdate<'a> {
    /// Moving back to `queued` also clears `last_error`.
    pub status: Option<PendingStatus>,
    pub url: Option<String>,
    pub tags: Option<&'a [TagName]>,
}

impl PendingUpdate<'_> {
    pub(crate) async fn apply(
        self,
        conn: &mut SqliteConnection,
        owner: i64,
        id: PendingId,
    ) -> Result<()> {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("UPDATE pending_items SET ");
        let mut sets = qb.separated(", ");
        let has_status = self.status.is_some();
        let has_url = self.url.is_some();

        if let Some(status) = self.status {
            sets.push("status = ").push_bind_unseparated(status);
            if status == PendingStatus::Queued {
                sets.push("last_error = NULL");
            }
        }

        if let Some(url) = self.url.as_deref() {
            sets.push("url = ").push_bind_unseparated(url);
        }

        if has_status || has_url {
            qb.push(" WHERE owner_user_id = ")
                .push_bind(owner)
                .push(" AND id = ")
                .push_bind(id);
            qb.build().execute(&mut *conn).await?;
        }

        if let Some(tags) = self.tags {
            PendingRow::replace_tags(&mut *conn, owner, id, tags).await?;
        }
        Ok(())
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Hydrated items and listing
// ───────────────────────────────────────────────────────────────────────────

#[derive(FromRow)]
struct PendingTagJoin {
    pending_id: PendingId,
    #[sqlx(flatten)]
    tag: Tag,
}

impl PendingResponse {
    pub(crate) async fn load(pool: &SqlitePool, owner: i64, id: PendingId) -> Result<Option<Self>> {
        let Some(record) = PendingRow::load(pool, owner, id).await? else {
            return Ok(None);
        };
        Ok(Self::hydrate(pool, vec![record]).await?.pop())
    }

    /// Like [`Self::load`], but reports a missing item as a typed `404`.
    pub(crate) async fn get(
        pool: &SqlitePool,
        owner: i64,
        id: PendingId,
    ) -> crate::error::Result<Self> {
        Self::load(pool, owner, id)
            .await?
            .ok_or(crate::error::AppError::NotFound(
                crate::error::Resource::PendingItem,
            ))
    }

    /// Attaches tags to many records using a single extra query.
    pub(crate) async fn hydrate<'e>(
        executor: impl SqliteExecutor<'e>,
        records: Vec<PendingRow>,
    ) -> Result<Vec<Self>> {
        if records.is_empty() {
            return Ok(Vec::new());
        }

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT pit.pending_item_id AS pending_id, t.id, t.name, t.color, t.created_at
             FROM pending_item_tags pit
             JOIN tags t ON t.id = pit.tag_id
             WHERE pit.pending_item_id IN (",
        );
        let mut ids = qb.separated(", ");
        for record in &records {
            ids.push_bind(record.id);
        }
        ids.push_unseparated(") ORDER BY t.name COLLATE NOCASE");

        let joins: Vec<PendingTagJoin> = qb.build_query_as().fetch_all(executor).await?;

        let mut tags_by_id: HashMap<PendingId, Vec<Tag>> = HashMap::new();
        for join in joins {
            tags_by_id
                .entry(join.pending_id)
                .or_default()
                .push(join.tag);
        }

        Ok(records
            .into_iter()
            .map(|record| Self {
                tags: tags_by_id.remove(&record.id).unwrap_or_default(),
                record,
            })
            .collect())
    }

    /// Newest-first keyset pagination for the queue.
    ///
    /// `after` is the cursor returned with the previous page.
    pub(crate) async fn list(
        pool: &SqlitePool,
        owner: i64,
        status: Option<PendingStatus>,
        after: Option<Cursor>,
        limit: Limit,
    ) -> Result<CursorPage<Self>> {
        let limit = i64::from(limit.get());

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(concat!(
            "SELECT ",
            pending_columns!(),
            " FROM pending_items WHERE owner_user_id = "
        ));
        qb.push_bind(owner);
        if let Some(after) = after {
            qb.push(" AND id < ").push_bind(after.get());
        }
        if let Some(status) = status {
            qb.push(" AND status = ").push_bind(status);
        }
        // Fetch one extra row to detect whether another page exists.
        qb.push(" ORDER BY id DESC LIMIT ")
            .push_bind(limit.saturating_add(1));

        let records: Vec<PendingRow> = qb.build_query_as().fetch_all(pool).await?;

        let page = CursorPage::from_overfetched(
            records,
            usize::try_from(limit).unwrap_or(usize::MAX),
            |record| Cursor::new(record.id.get()),
        );
        let items = Self::hydrate(pool, page.items).await?;

        Ok(CursorPage {
            items,
            next_cursor: page.next_cursor,
        })
    }
}
