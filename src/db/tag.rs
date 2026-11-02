use serde::Serialize;
use sqlx::{FromRow, Result, SqliteExecutor, SqlitePool};

use crate::{
    db::helper::{CreateOutcome, FieldPatch},
    tag::{TagColor, TagId, TagMerge, TagName},
};

/// Column list shared by every query that materializes a [`Tag`].
macro_rules! tag_columns {
    () => {
        "id, name, color, created_at"
    };
}

// ───────────────────────────────────────────────────────────────────────────
// Models
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Tag {
    pub id: TagId,
    pub name: TagName,
    /// Palette key, never a hex value.
    pub color: Option<TagColor>,
    pub created_at: i64,
}

/// A tag plus how many pages and pending items reference it.
#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagSummary {
    pub id: TagId,
    pub name: TagName,
    pub color: Option<TagColor>,
    pub created_at: i64,
    pub page_count: i64,
    pub pending_count: i64,
}

// ───────────────────────────────────────────────────────────────────────────
// Queries
// ───────────────────────────────────────────────────────────────────────────

impl TagSummary {
    /// All tags, most-used first, then alphabetical.
    pub(crate) async fn list(pool: &SqlitePool) -> Result<Vec<Self>> {
        sqlx::query_as(
            "SELECT t.id, t.name, t.color, t.created_at,
                    (SELECT COUNT(*) FROM page_tag pgt    WHERE pgt.tag_id = t.id) AS page_count,
                    (SELECT COUNT(*) FROM pending_tag pt WHERE pt.tag_id = t.id) AS pending_count
             FROM tag t
             ORDER BY page_count DESC, t.name COLLATE NOCASE",
        )
        .fetch_all(pool)
        .await
    }

    pub(crate) async fn find_by_name(pool: &SqlitePool, name: &TagName) -> Result<Option<Self>> {
        sqlx::query_as(
            "SELECT t.id, t.name, t.color, t.created_at,
                    (SELECT COUNT(*) FROM page_tag pgt    WHERE pgt.tag_id = t.id) AS page_count,
                    (SELECT COUNT(*) FROM pending_tag pt WHERE pt.tag_id = t.id) AS pending_count
             FROM tag t WHERE t.name = ?",
        )
        .bind(name)
        .fetch_optional(pool)
        .await
    }
}

impl Tag {
    pub(crate) async fn load<'e>(
        executor: impl SqliteExecutor<'e>,
        id: TagId,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!("SELECT ", tag_columns!(), " FROM tag WHERE id = ?"))
            .bind(id)
            .fetch_optional(executor)
            .await
    }

    /// Id of the tag named `name`, ignoring the tag `excluding` (useful for rename checks).
    pub(crate) async fn find_id_by_name<'e>(
        executor: impl SqliteExecutor<'e>,
        name: &TagName,
        excluding: Option<TagId>,
    ) -> Result<Option<TagId>> {
        sqlx::query_scalar("SELECT id FROM tag WHERE name = ? AND id IS NOT ?")
            .bind(name)
            .bind(excluding)
            .fetch_optional(executor)
            .await
    }

    /// Creates the tag, or returns the existing one if the name is taken.
    ///
    /// On [`CreateOutcome::Existing`] the stored row is returned as-is: the
    /// requested `color` is *not* applied to it.
    ///
    /// Runs in one transaction. The insert is the first statement, so SQLite
    /// takes the write lock up front, and the follow-up `SELECT` cannot race
    /// with a concurrent delete or rename of the conflicting row.
    pub(crate) async fn create(
        pool: &SqlitePool,
        name: &TagName,
        color: Option<&TagColor>,
    ) -> Result<CreateOutcome<Self>> {
        let mut tx = pool.begin().await?;

        let inserted: Option<Self> = sqlx::query_as(concat!(
            "INSERT INTO tag (name, color) VALUES (?, ?)
             ON CONFLICT(name) DO NOTHING
             RETURNING ",
            tag_columns!()
        ))
        .bind(name)
        .bind(color)
        .fetch_optional(&mut *tx)
        .await?;

        let outcome = if let Some(tag) = inserted {
            CreateOutcome::Created(tag)
        } else {
            let existing = sqlx::query_as(concat!(
                "SELECT ",
                tag_columns!(),
                " FROM tag WHERE name = ?"
            ))
            .bind(name)
            .fetch_one(&mut *tx)
            .await?;
            CreateOutcome::Existing(existing)
        };

        tx.commit().await?;
        Ok(outcome)
    }

    /// Returns the id of the tag named `name`, creating it if necessary.
    pub(crate) async fn get_or_create_id<'e>(
        executor: impl SqliteExecutor<'e>,
        name: &TagName,
    ) -> Result<TagId> {
        // No-op DO UPDATE so RETURNING yields the existing row, keeping its original casing.
        sqlx::query_scalar(
            "INSERT INTO tag (name) VALUES (?)
             ON CONFLICT(name) DO UPDATE SET name = name
             RETURNING id",
        )
        .bind(name)
        .fetch_one(executor)
        .await
    }

    pub(crate) async fn rename(
        pool: &SqlitePool,
        id: TagId,
        name: &TagName,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "UPDATE tag SET name = ? WHERE id = ? RETURNING ",
            tag_columns!()
        ))
        .bind(name)
        .bind(id)
        .fetch_optional(pool)
        .await
    }

    /// Returns `true` if a tag was deleted.
    pub(crate) async fn delete(pool: &SqlitePool, id: TagId) -> Result<bool> {
        let result = sqlx::query("DELETE FROM tag WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Moves every association from `merge.source()` to `merge.target()`, then
    /// deletes the source. Returns the updated target, or `None` if either tag
    /// does not exist.
    pub(crate) async fn merge(pool: &SqlitePool, merge: TagMerge) -> Result<Option<Self>> {
        let (source, target) = (merge.source(), merge.target());
        let mut tx = pool.begin().await?;

        // `merge` guarantees source != target, so both exist iff the count is 2.
        let found: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tag WHERE id IN (?, ?)")
            .bind(source)
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
        if found != 2 {
            return Ok(None);
        }

        sqlx::query(
            "INSERT OR IGNORE INTO page_tag (page_id, tag_id)
             SELECT page_id, ? FROM page_tag WHERE tag_id = ?",
        )
        .bind(target)
        .bind(source)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT OR IGNORE INTO pending_tag (pending_id, tag_id)
             SELECT pending_id, ? FROM pending_tag WHERE tag_id = ?",
        )
        .bind(target)
        .bind(source)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM tag WHERE id = ?")
            .bind(source)
            .execute(&mut *tx)
            .await?;

        let merged = Self::load(&mut *tx, target).await?;
        tx.commit().await?;
        Ok(merged)
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Patch
// ───────────────────────────────────────────────────────────────────────────

/// PATCH semantics: absent fields are left unchanged; `color: null` clears it.
#[derive(Debug, Default)]
pub(crate) struct TagPatch {
    pub name: Option<TagName>,
    pub color: FieldPatch<TagColor>,
}

#[derive(Debug)]
pub(crate) enum PatchOutcome {
    Updated(Tag),
    NotFound,
    /// The requested name belongs to a different tag.
    NameTaken {
        existing_id: TagId,
    },
}

impl TagPatch {
    pub(crate) async fn apply(self, pool: &SqlitePool, id: TagId) -> Result<PatchOutcome> {
        let mut tx = pool.begin().await?;

        let Some(current) = Tag::load(&mut *tx, id).await? else {
            return Ok(PatchOutcome::NotFound);
        };

        let name = self.name.unwrap_or(current.name);
        let color = self.color.apply(current.color);

        if let Some(existing_id) = Tag::find_id_by_name(&mut *tx, &name, Some(id)).await? {
            return Ok(PatchOutcome::NameTaken { existing_id });
        }

        let updated: Tag = sqlx::query_as(concat!(
            "UPDATE tag SET name = ?, color = ? WHERE id = ? RETURNING ",
            tag_columns!()
        ))
        .bind(&name)
        .bind(&color)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(PatchOutcome::Updated(updated))
    }
}
