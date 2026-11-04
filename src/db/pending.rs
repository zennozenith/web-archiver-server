use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Result, Sqlite, SqliteConnection, SqliteExecutor, SqlitePool};

use crate::{
    db::{
        helper::{CreateOutcome, Cursor, CursorPage, id_type},
        tag::Tag,
    },
    limit::Limit,
    tag::TagName,
    url_canon::CanonicalUrl,
};

/// Column list shared by every query that materializes a [`PendingRecord`].
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, strum::Display)]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub(crate) enum PendingStatus {
    Queued,
    InProgress,
    Failed,
}

/// A row of the `pending` table.
#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingRecord {
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
pub(crate) struct PendingItem {
    #[serde(flatten)]
    pub record: PendingRecord,
    pub tags: Vec<Tag>,
}

// ───────────────────────────────────────────────────────────────────────────
// Single-record operations
// ───────────────────────────────────────────────────────────────────────────

impl PendingRecord {
    pub(crate) async fn load<'e>(
        executor: impl SqliteExecutor<'e>,
        id: PendingId,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "SELECT ",
            pending_columns!(),
            " FROM pending WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(executor)
        .await
    }

    pub(crate) async fn exists<'e>(
        executor: impl SqliteExecutor<'e>,
        id: PendingId,
    ) -> Result<bool> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pending WHERE id = ?)")
            .bind(id)
            .fetch_one(executor)
            .await
    }

    /// Current status, or `None` if the item does not exist.
    pub(crate) async fn status_of<'e>(
        executor: impl SqliteExecutor<'e>,
        id: PendingId,
    ) -> Result<Option<PendingStatus>> {
        sqlx::query_scalar("SELECT status FROM pending WHERE id = ?")
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
        url: &CanonicalUrl,
        tags: &[TagName],
    ) -> Result<CreateOutcome<PendingId>> {
        let inserted: Option<PendingId> = sqlx::query_scalar(
            "INSERT INTO pending (url) VALUES (?, ?)
             ON CONFLICT(url) DO NOTHING
             RETURNING id",
        )
        .bind(&url.url)
        .fetch_optional(&mut *conn)
        .await?;

        if let Some(id) = inserted {
            Self::replace_tags(&mut *conn, id, tags).await?;
            return Ok(CreateOutcome::Created(id));
        }

        // TODO: merge the requested tags into the existing item.
        let id = sqlx::query_scalar("SELECT id FROM pending WHERE url = ?")
            .bind(&url.url)
            .fetch_one(&mut *conn)
            .await?;
        Ok(CreateOutcome::Existing(id))
    }

    /// Replaces the item's tag set with `names`, creating missing tags.
    pub(crate) async fn replace_tags(
        conn: &mut SqliteConnection,
        id: PendingId,
        names: &[TagName],
    ) -> Result<()> {
        sqlx::query("DELETE FROM pending_tag WHERE pending_id = ?")
            .bind(id)
            .execute(&mut *conn)
            .await?;

        for name in names {
            let tag_id = Tag::get_or_create_id(&mut *conn, name).await?;
            sqlx::query("INSERT OR IGNORE INTO pending_tag (pending_id, tag_id) VALUES (?, ?)")
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
    pub(crate) async fn claim_next<'e>(
        executor: impl SqliteExecutor<'e>,
    ) -> Result<Option<PendingId>> {
        sqlx::query_scalar(
            "UPDATE pending SET status = 'in_progress'
             WHERE id = (SELECT id FROM pending WHERE status = 'queued'
                         ORDER BY created_at, id LIMIT 1)
             RETURNING id",
        )
        .fetch_optional(executor)
        .await
    }

    /// Moves a `failed` item back to `queued`. Returns `false` if it was not failed.
    pub(crate) async fn retry<'e>(
        executor: impl SqliteExecutor<'e>,
        id: PendingId,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE pending SET status = 'queued', last_error = NULL
             WHERE id = ? AND status = 'failed'",
        )
        .bind(id)
        .execute(executor)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Marks an `in_progress` item as failed. Returns `false` if it was not in progress.
    pub(crate) async fn mark_failed<'e>(
        executor: impl SqliteExecutor<'e>,
        id: PendingId,
        error: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE pending SET status = 'failed', attempts = attempts + 1, last_error = ?
             WHERE id = ? AND status = 'in_progress'",
        )
        .bind(error)
        .bind(id)
        .execute(executor)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Deletes an item unless a worker is processing it. Returns `true` if deleted.
    pub(crate) async fn delete<'e>(
        executor: impl SqliteExecutor<'e>,
        id: PendingId,
    ) -> Result<bool> {
        let result = sqlx::query("DELETE FROM pending WHERE id = ? AND status != 'in_progress'")
            .bind(id)
            .execute(executor)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Deletes every item with `status`; returns how many were removed.
    pub(crate) async fn delete_all_with_status<'e>(
        executor: impl SqliteExecutor<'e>,
        status: PendingStatus,
    ) -> Result<u64> {
        let result = sqlx::query("DELETE FROM pending WHERE status = ?")
            .bind(status)
            .execute(executor)
            .await?;
        Ok(result.rows_affected())
    }

    /// Call once at startup: anything left `in_progress` belongs to a dead worker.
    #[expect(
        dead_code,
        reason = "startup recovery hook for deployments with external workers"
    )]
    pub(crate) async fn requeue_stalled<'e>(executor: impl SqliteExecutor<'e>) -> Result<u64> {
        let result =
            sqlx::query("UPDATE pending SET status = 'queued' WHERE status = 'in_progress'")
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
    pub tags: Option<&'a [TagName]>,
}

impl PendingUpdate<'_> {
    pub(crate) async fn apply(self, conn: &mut SqliteConnection, id: PendingId) -> Result<()> {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("UPDATE pending SET ");
        let mut sets = qb.separated(", ");
        let has_sets = self.status.is_some_and(|status| {
            sets.push("status = ").push_bind_unseparated(status);
            if status == PendingStatus::Queued {
                sets.push("last_error = NULL");
            }
            true
        });

        if has_sets {
            qb.push(" WHERE id = ").push_bind(id);
            qb.build().execute(&mut *conn).await?;
        }

        if let Some(tags) = self.tags {
            PendingRecord::replace_tags(&mut *conn, id, tags).await?;
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

impl PendingItem {
    pub(crate) async fn load(pool: &SqlitePool, id: PendingId) -> Result<Option<Self>> {
        let Some(record) = PendingRecord::load(pool, id).await? else {
            return Ok(None);
        };
        Ok(Self::hydrate(pool, vec![record]).await?.pop())
    }

    /// Attaches tags to many records using a single extra query.
    pub(crate) async fn hydrate<'e>(
        executor: impl SqliteExecutor<'e>,
        records: Vec<PendingRecord>,
    ) -> Result<Vec<Self>> {
        if records.is_empty() {
            return Ok(Vec::new());
        }

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT pt.pending_id, t.id, t.name, t.color, t.created_at
             FROM pending_tag pt
             JOIN tag t ON t.id = pt.tag_id
             WHERE pt.pending_id IN (",
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

    /// Newest-first keyset pagination for `GET /api/pending`.
    ///
    /// `after` is the cursor returned with the previous page.
    pub(crate) async fn list(
        pool: &SqlitePool,
        status: Option<PendingStatus>,
        after: Option<Cursor>,
        limit: Limit,
    ) -> Result<CursorPage<Self>> {
        let limit = i64::from(limit.get());

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(concat!(
            "SELECT ",
            pending_columns!(),
            " FROM pending WHERE 1 = 1"
        ));
        if let Some(after) = after {
            qb.push(" AND id < ").push_bind(after.get());
        }
        if let Some(status) = status {
            qb.push(" AND status = ").push_bind(status);
        }
        // Fetch one extra row to detect whether another page exists.
        qb.push(" ORDER BY id DESC LIMIT ")
            .push_bind(limit.saturating_add(1));

        let records: Vec<PendingRecord> = qb.build_query_as().fetch_all(pool).await?;

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
