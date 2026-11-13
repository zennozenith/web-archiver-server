use serde::Serialize;
use sqlx::{FromRow, Result, SqliteExecutor, SqlitePool};

use crate::{
    db::helper::{CreateOutcome, FieldPatch},
    infra::{TagColor, TagId, TagMerge, TagName},
};

/// Column list shared by every query that materializes a [`Tag`].
macro_rules! tag_columns {
    () => {
        "t.id, t.name, t.color, t.created_at"
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
    /// All of the owner's tags, most-used first, then alphabetical.
    pub(crate) async fn list(pool: &SqlitePool, owner: i64) -> Result<Vec<Self>> {
        sqlx::query_as(
            "SELECT t.id, t.name, t.color, t.created_at,
                    (SELECT COUNT(*) FROM page_tags pt     WHERE pt.tag_id = t.id) AS page_count,
                    (SELECT COUNT(*) FROM pending_item_tags pit WHERE pit.tag_id = t.id) AS pending_count
             FROM tags t
             WHERE t.owner_user_id = ?
             ORDER BY page_count DESC, t.name COLLATE NOCASE",
        )
        .bind(owner)
        .fetch_all(pool)
        .await
    }

    pub(crate) async fn find_by_name(
        pool: &SqlitePool,
        owner: i64,
        name: &TagName,
    ) -> Result<Option<Self>> {
        sqlx::query_as(
            "SELECT t.id, t.name, t.color, t.created_at,
                    (SELECT COUNT(*) FROM page_tags pt     WHERE pt.tag_id = t.id) AS page_count,
                    (SELECT COUNT(*) FROM pending_item_tags pit WHERE pit.tag_id = t.id) AS pending_count
             FROM tags t WHERE t.owner_user_id = ? AND t.name = ?",
        )
        .bind(owner)
        .bind(name)
        .fetch_optional(pool)
        .await
    }
}

impl Tag {
    pub(crate) async fn load<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        id: TagId,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "SELECT ",
            tag_columns!(),
            " FROM tags t WHERE t.owner_user_id = ? AND t.id = ?"
        ))
        .bind(owner)
        .bind(id)
        .fetch_optional(executor)
        .await
    }

    /// Id of the owner's tag named `name`, ignoring the tag `excluding`.
    pub(crate) async fn find_id_by_name<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        name: &TagName,
        excluding: Option<TagId>,
    ) -> Result<Option<TagId>> {
        sqlx::query_scalar(
            "SELECT id FROM tags
             WHERE owner_user_id = ? AND name = ? AND id IS NOT ?",
        )
        .bind(owner)
        .bind(name)
        .bind(excluding)
        .fetch_optional(executor)
        .await
    }

    /// Creates the tag, or returns the existing one if the name is taken.
    pub(crate) async fn create(
        pool: &SqlitePool,
        owner: i64,
        name: &TagName,
        color: Option<&TagColor>,
    ) -> Result<CreateOutcome<Self>> {
        let mut tx = pool.begin().await?;

        let inserted: Option<Self> = sqlx::query_as(
            "INSERT INTO tags (owner_user_id, name, color) VALUES (?, ?, ?)
             ON CONFLICT(owner_user_id, name) DO NOTHING
             RETURNING id, name, color, created_at",
        )
        .bind(owner)
        .bind(name)
        .bind(color)
        .fetch_optional(&mut *tx)
        .await?;

        let outcome = if let Some(tag) = inserted {
            CreateOutcome::Created(tag)
        } else {
            let existing: Self = sqlx::query_as(concat!(
                "SELECT ",
                tag_columns!(),
                " FROM tags t WHERE t.owner_user_id = ? AND t.name = ?"
            ))
            .bind(owner)
            .bind(name)
            .fetch_one(&mut *tx)
            .await?;
            CreateOutcome::Existing(existing)
        };

        tx.commit().await?;
        Ok(outcome)
    }

    /// Returns the id of the owner's tag named `name`, creating it if necessary.
    pub(crate) async fn get_or_create_id<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        name: &TagName,
    ) -> Result<TagId> {
        // No-op DO UPDATE so RETURNING yields the existing row, keeping its original casing.
        sqlx::query_scalar(
            "INSERT INTO tags (owner_user_id, name) VALUES (?, ?)
             ON CONFLICT(owner_user_id, name) DO UPDATE SET name = name
             RETURNING id",
        )
        .bind(owner)
        .bind(name)
        .fetch_one(executor)
        .await
    }

    pub(crate) async fn rename(
        pool: &SqlitePool,
        owner: i64,
        id: TagId,
        name: &TagName,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            "UPDATE tags SET name = ? WHERE owner_user_id = ? AND id = ? RETURNING ",
            tag_columns!()
        ))
        .bind(name)
        .bind(owner)
        .bind(id)
        .fetch_optional(pool)
        .await
    }

    /// Returns `true` if a tag was deleted.
    pub(crate) async fn delete(pool: &SqlitePool, owner: i64, id: TagId) -> Result<bool> {
        let result = sqlx::query("DELETE FROM tags WHERE owner_user_id = ? AND id = ?")
            .bind(owner)
            .bind(id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Moves every association from `merge.source()` to `merge.target()`, then
    /// deletes the source. Returns the updated target, or `None` if either tag
    /// does not exist.
    pub(crate) async fn merge(
        pool: &SqlitePool,
        owner: i64,
        merge: TagMerge,
    ) -> Result<Option<Self>> {
        let (source, target) = (merge.source(), merge.target());
        let mut tx = pool.begin().await?;

        // `merge` guarantees source != target, so both exist iff the count is 2.
        let found: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tags WHERE owner_user_id = ? AND id IN (?, ?)",
        )
        .bind(owner)
        .bind(source)
        .bind(target)
        .fetch_one(&mut *tx)
        .await?;
        if found != 2 {
            return Ok(None);
        }

        sqlx::query(
            "INSERT OR IGNORE INTO page_tags (page_id, tag_id)
             SELECT pt.page_id, ? FROM page_tags pt
             JOIN pages p ON p.id = pt.page_id
             WHERE pt.tag_id = ? AND p.owner_user_id = ?",
        )
        .bind(target)
        .bind(source)
        .bind(owner)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT OR IGNORE INTO pending_item_tags (pending_item_id, tag_id)
             SELECT pit.pending_item_id, ? FROM pending_item_tags pit
             JOIN pending_items pi ON pi.id = pit.pending_item_id
             WHERE pit.tag_id = ? AND pi.owner_user_id = ?",
        )
        .bind(target)
        .bind(source)
        .bind(owner)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM tags WHERE owner_user_id = ? AND id = ?")
            .bind(owner)
            .bind(source)
            .execute(&mut *tx)
            .await?;

        let merged = Self::load(&mut *tx, owner, target).await?;
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
    pub(crate) async fn apply(
        self,
        pool: &SqlitePool,
        owner: i64,
        id: TagId,
    ) -> Result<PatchOutcome> {
        let mut tx = pool.begin().await?;

        let Some(current) = Tag::load(&mut *tx, owner, id).await? else {
            return Ok(PatchOutcome::NotFound);
        };

        let name = self.name.unwrap_or(current.name);
        let color = self.color.apply(current.color);

        if let Some(existing_id) = Tag::find_id_by_name(&mut *tx, owner, &name, Some(id)).await? {
            return Ok(PatchOutcome::NameTaken { existing_id });
        }

        let updated: Tag = sqlx::query_as(
            "UPDATE tags SET name = ?, color = ?
             WHERE owner_user_id = ? AND id = ?
             RETURNING id, name, color, created_at",
        )
        .bind(&name)
        .bind(&color)
        .bind(owner)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(PatchOutcome::Updated(updated))
    }
}
