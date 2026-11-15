//! Collections: flat, per-user folders that group links.

use serde::Serialize;
use sqlx::{FromRow, Result, SqliteExecutor, SqlitePool};

use crate::{domain::ids::id_type, error::AppError};

id_type! {
    /// Primary key of a collection.
    pub(crate) struct CollectionId;
}

impl CollectionId {
    pub(crate) const fn get(self) -> i64 {
        self.0
    }

    pub(crate) fn parse_opt(value: Option<&str>) -> Option<Self> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|id| *id >= 0)
            .map(Self::from)
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Models
// ───────────────────────────────────────────────────────────────────────────

/// A collection together with the number of links filed directly under it.
#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CollectionRecord {
    pub id: CollectionId,
    pub owner_user_id: i64,
    pub name: String,
    pub emoji: String,
    pub description: String,
    pub color: String,
    pub public: bool,
    pub is_builtin: bool,
    pub created_at: i64,
    pub link_count: i64,
}

/// Fields accepted when creating or editing a collection.
#[derive(Debug)]
pub(crate) struct CollectionInput<'a> {
    pub name: &'a str,
    pub emoji: &'a str,
    pub description: &'a str,
    pub color: &'a str,
    pub public: bool,
}

// ───────────────────────────────────────────────────────────────────────────
// Reads
// ───────────────────────────────────────────────────────────────────────────

macro_rules! collection_select {
    () => {
        "SELECT c.id, c.owner_user_id, c.name, c.emoji, c.description, c.color, c.public,
                c.is_builtin, c.created_at,
                (SELECT COUNT(*) FROM pages p WHERE p.collection_id = c.id) AS link_count
         FROM collections c"
    };
}

impl CollectionRecord {
    pub(crate) async fn list(pool: &SqlitePool, owner: i64) -> Result<Vec<Self>> {
        sqlx::query_as(concat!(
            collection_select!(),
            " WHERE c.owner_user_id = ?
              ORDER BY c.is_builtin DESC, c.name COLLATE NOCASE, c.id"
        ))
        .bind(owner)
        .fetch_all(pool)
        .await
    }

    pub(crate) async fn load<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        id: CollectionId,
    ) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            collection_select!(),
            " WHERE c.owner_user_id = ? AND c.id = ?"
        ))
        .bind(owner)
        .bind(id)
        .fetch_optional(executor)
        .await
    }

    /// Loads a public collection without knowing its owner.
    pub(crate) async fn load_public(pool: &SqlitePool, id: CollectionId) -> Result<Option<Self>> {
        sqlx::query_as(concat!(
            collection_select!(),
            " WHERE c.id = ? AND c.public = 1"
        ))
        .bind(id)
        .fetch_optional(pool)
        .await
    }

    pub(crate) async fn exists<'e>(
        executor: impl SqliteExecutor<'e>,
        owner: i64,
        id: CollectionId,
    ) -> Result<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM collections WHERE owner_user_id = ? AND id = ?)",
        )
        .bind(owner)
        .bind(id)
        .fetch_one(executor)
        .await
    }

    // ───────────────────────────────────────────────────────────────────────
    // Mutations
    // ───────────────────────────────────────────────────────────────────────

    pub(crate) async fn create(
        pool: &SqlitePool,
        owner: i64,
        input: &CollectionInput<'_>,
    ) -> Result<Self> {
        let id: CollectionId = sqlx::query_scalar(
            "INSERT INTO collections (owner_user_id, name, emoji, description, color, public)
             VALUES (?, ?, ?, ?, ?, ?)
             RETURNING id",
        )
        .bind(owner)
        .bind(input.name)
        .bind(input.emoji)
        .bind(input.description)
        .bind(input.color)
        .bind(input.public)
        .fetch_one(pool)
        .await?;

        Self::load(pool, owner, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    /// Returns `None` if the collection does not exist.
    pub(crate) async fn update(
        pool: &SqlitePool,
        owner: i64,
        id: CollectionId,
        input: &CollectionInput<'_>,
    ) -> Result<Option<Self>> {
        let updated: Option<CollectionId> = sqlx::query_scalar(
            "UPDATE collections
             SET name = ?, emoji = ?, description = ?, color = ?, public = ?
             WHERE owner_user_id = ? AND id = ?
             RETURNING id",
        )
        .bind(input.name)
        .bind(input.emoji)
        .bind(input.description)
        .bind(input.color)
        .bind(input.public)
        .bind(owner)
        .bind(id)
        .fetch_optional(pool)
        .await?;

        match updated {
            Some(updated_id) => Self::load(pool, owner, updated_id).await,
            None => Ok(None),
        }
    }

    /// Deletes the collection. Pages filed in it become unorganized through the
    /// `ON DELETE SET NULL` foreign key. The built-in collection is protected.
    pub(crate) async fn delete(
        pool: &SqlitePool,
        owner: i64,
        id: CollectionId,
    ) -> Result<bool, AppError> {
        let Some(record) = Self::load(pool, owner, id).await? else {
            return Ok(false);
        };
        if record.is_builtin {
            return Err(AppError::Conflict(
                "the built-in Archive collection cannot be deleted".into(),
            ));
        }

        let result = sqlx::query("DELETE FROM collections WHERE owner_user_id = ? AND id = ?")
            .bind(owner)
            .bind(id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub(crate) async fn set_public(
        pool: &SqlitePool,
        owner: i64,
        id: CollectionId,
        public: bool,
    ) -> Result<bool> {
        let result =
            sqlx::query("UPDATE collections SET public = ? WHERE owner_user_id = ? AND id = ?")
                .bind(public)
                .bind(owner)
                .bind(id)
                .execute(pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }
}
