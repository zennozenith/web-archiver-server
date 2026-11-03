//! Collections: nested folders that group links.

use sqlx::{FromRow, Result, SqliteExecutor, SqlitePool};

use crate::db::helper::id_type;

id_type! {
    /// Primary key of a collection.
    pub(crate) struct CollectionId;
}

// ───────────────────────────────────────────────────────────────────────────
// Models
// ───────────────────────────────────────────────────────────────────────────

/// A collection together with the number of links filed directly under it.
#[derive(Debug, FromRow)]
pub(crate) struct CollectionRecord {
    pub id: CollectionId,
    pub name: String,
    pub emoji: String,
    pub description: String,
    pub color: String,
    pub parent_id: Option<CollectionId>,
    pub public: bool,
    pub link_count: i64,
}

/// Fields accepted when creating or editing a collection.
#[derive(Debug)]
pub(crate) struct CollectionInput<'a> {
    pub name: &'a str,
    pub emoji: &'a str,
    pub description: &'a str,
    pub color: &'a str,
    pub parent_id: Option<CollectionId>,
    pub public: bool,
}

// ───────────────────────────────────────────────────────────────────────────
// Reads
// ───────────────────────────────────────────────────────────────────────────

macro_rules! collection_select {
    () => {
        "SELECT c.id, c.name, c.emoji, c.description, c.color, c.parent_id,
                c.public, c.created_at,
                (SELECT COUNT(*) FROM page p WHERE p.collection_id = c.id) AS link_count
         FROM collection c"
    };
}

impl CollectionRecord {
    pub(crate) async fn list(pool: &SqlitePool) -> Result<Vec<Self>> {
        sqlx::query_as(concat!(
            collection_select!(),
            " ORDER BY c.name COLLATE NOCASE, c.id"
        ))
        .fetch_all(pool)
        .await
    }

    pub(crate) async fn load<'e>(
        executor: impl SqliteExecutor<'e>,
        id: CollectionId,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(collection_select!(), " WHERE c.id = ?"))
            .bind(id)
            .fetch_optional(executor)
            .await
    }

    pub(crate) async fn exists<'e>(
        executor: impl SqliteExecutor<'e>,
        id: CollectionId,
    ) -> Result<bool> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM collection WHERE id = ?)")
            .bind(id)
            .fetch_one(executor)
            .await
    }

    // ───────────────────────────────────────────────────────────────────────
    // Mutations
    // ───────────────────────────────────────────────────────────────────────

    pub(crate) async fn create(pool: &SqlitePool, input: &CollectionInput<'_>) -> Result<Self> {
        let id: CollectionId = sqlx::query_scalar(
            "INSERT INTO collection (name, emoji, description, color, parent_id, public)
             VALUES (?, ?, ?, ?, ?, ?)
             RETURNING id",
        )
        .bind(input.name)
        .bind(input.emoji)
        .bind(input.description)
        .bind(input.color)
        .bind(input.parent_id)
        .bind(input.public)
        .fetch_one(pool)
        .await?;

        Self::load(pool, id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    // TODO: if needed, partial update could be implemented. See PendingUpdate
    /// Returns `None` if the collection does not exist.
    pub(crate) async fn update(
        pool: &SqlitePool,
        id: CollectionId,
        input: &CollectionInput<'_>,
    ) -> Result<Option<Self>> {
        let updated: Option<CollectionId> = sqlx::query_scalar(
            "UPDATE collection
             SET name = ?, emoji = ?, description = ?, color = ?, parent_id = ?, public = ?
             WHERE id = ?
             RETURNING id",
        )
        .bind(input.name)
        .bind(input.emoji)
        .bind(input.description)
        .bind(input.color)
        .bind(input.parent_id)
        .bind(input.public)
        .bind(id)
        .fetch_optional(pool)
        .await?;

        match updated {
            Some(updated_id) => Self::load(pool, updated_id).await,
            None => Ok(None),
        }
    }

    /// Deletes the collection, the links filed directly under it, and detaches
    /// any child collections. Returns `false` if the collection did not exist.
    pub(crate) async fn delete(pool: &SqlitePool, id: CollectionId) -> Result<bool> {
        let mut tx = pool.begin().await?;

        // Captures of other pages that redirected into this collection stay,
        // but lose the dangling reference.
        sqlx::query(
            "UPDATE capture SET redirect_page_id = NULL
             WHERE redirect_page_id IN (SELECT id FROM page WHERE collection_id = ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM capture WHERE page_id IN (SELECT id FROM page WHERE collection_id = ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM page WHERE collection_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("UPDATE collection SET parent_id = NULL WHERE parent_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        let deleted = sqlx::query("DELETE FROM collection WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;

        tx.commit().await?;
        Ok(deleted)
    }

    pub(crate) async fn set_public(
        pool: &SqlitePool,
        id: CollectionId,
        public: bool,
    ) -> Result<bool> {
        let result = sqlx::query("UPDATE collection SET public = ? WHERE id = ?")
            .bind(public)
            .bind(id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
