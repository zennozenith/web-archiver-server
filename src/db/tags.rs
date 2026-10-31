use serde::Serialize;
use sqlx::{Result, SqliteConnection, SqlitePool, prelude::FromRow};

use crate::{
    db::helpers::Inserted,
    tag::{Color, Merge, Name},
};

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagWithCount {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub created_at: i64,
    pub pages: i64,
    pub pending: i64,
}

impl TagWithCount {
    pub(crate) async fn list(pool: &SqlitePool) -> Result<Vec<Self>> {
        let rows = sqlx::query_as(
            "SELECT t.id, t.name, t.color, t.created_at,
                (SELECT COUNT(*) FROM url_tag ut WHERE ut.tag_id = t.id)     AS pages,
                (SELECT COUNT(*) FROM pending_tag pt WHERE pt.tag_id = t.id) AS pending
         FROM tag t
         ORDER BY pages DESC, t.name COLLATE NOCASE",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagRow {
    pub id: i64,
    pub name: Name,
    pub color: Option<Color>, // palette key, never a hex value
    pub created_at: i64,
}

impl TagRow {
    pub(crate) async fn create(
        pool: &SqlitePool,
        name: Name,
        color: Option<Color>,
    ) -> Result<Inserted<Self>> {
        let created: Option<Self> = sqlx::query_as(
            "INSERT INTO tag (name, color) VALUES (?, ?)
         ON CONFLICT(name) DO NOTHING
         RETURNING id, name, color, created_at",
        )
        .bind(&name)
        .bind(&color)
        .fetch_optional(pool)
        .await?;

        if let Some(tag) = created {
            Ok(Inserted::New(tag))
        } else {
            let existing_id: i64 = sqlx::query_scalar("SELECT id FROM tag WHERE name = ?")
                .bind(&name)
                .fetch_one(pool)
                .await?;
            Ok(Inserted::Existing(existing_id))
        }
    }

    pub(crate) async fn load(conn: &mut SqliteConnection, id: i64) -> Result<Option<Self>> {
        sqlx::query_as("SELECT id, name, color, created_at FROM tag WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await
    }

    pub(crate) async fn delete(pool: &SqlitePool, id: i64) -> Result<u64> {
        let n = sqlx::query("DELETE FROM tag WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected();

        Ok(n)
    }

    pub(crate) async fn merge(pool: &SqlitePool, merge: Merge) -> Result<Option<Self>> {
        let mut tx = pool.begin().await?;

        let from = merge.from();
        let into = merge.into_();
        let found: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tag WHERE id IN (?, ?)")
            .bind(from)
            .bind(into)
            .fetch_one(&mut *tx)
            .await?;
        if found != 2 {
            return Ok(None);
        }

        sqlx::query("INSERT OR IGNORE INTO url_tag (url_id, tag_id) SELECT url_id, ? FROM url_tag WHERE tag_id = ?")
        .bind(into)
        .bind(from)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
        "INSERT OR IGNORE INTO pending_tag (pending_id, tag_id) SELECT pending_id, ? FROM pending_tag WHERE tag_id = ?",
        )
        .bind(into)
        .bind(from)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM tag WHERE id = ?")
            .bind(from)
            .execute(&mut *tx)
            .await?;

        let target = Self::load(&mut tx, into).await?;
        tx.commit().await?;
        Ok(target)
    }

    pub(crate) async fn exists(
        pool: &SqlitePool,
        name: &Name,
        except_id: Option<i64>,
    ) -> Result<Option<i64>> {
        sqlx::query_scalar("SELECT id FROM tag WHERE name = ? AND id IS NOT ?")
            .bind(name)
            .bind(except_id)
            .fetch_optional(pool)
            .await
    }

    pub(crate) async fn rename(pool: &SqlitePool, id: i64, name: &Name) -> Result<Option<Self>> {
        sqlx::query_as("UPDATE tag SET name = ? WHERE id = ? RETURNING id, name, color, created_at")
            .bind(name)
            .bind(id)
            .fetch_optional(pool)
            .await
    }
}

/// PATCH semantics: `color: null` clears, absent leaves unchanged.
#[derive(Debug)]
#[expect(
    clippy::option_option,
    reason = "distinguishes an absent field from an explicit null in PATCH payloads"
)]
pub(crate) struct PatchTag {
    pub name: Option<Name>,
    pub color: Option<Option<Color>>,
}

impl PatchTag {
    pub(crate) async fn patch(
        self,
        pool: &SqlitePool,
        id: i64,
    ) -> Result<Option<Inserted<TagRow>>> {
        let mut tx = pool.begin().await?;
        let Some(current) = TagRow::load(&mut tx, id).await? else {
            return Ok(None);
        };

        let name = self.name.unwrap_or(current.name);
        let color = self.color.unwrap_or(current.color);

        let clash: Option<i64> =
            sqlx::query_scalar("SELECT id FROM tag WHERE name = ? AND id != ?")
                .bind(&name)
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;

        if let Some(existing_id) = clash {
            return Ok(Some(Inserted::Existing(existing_id)));
        }

        sqlx::query("UPDATE tag SET name = ?, color = ? WHERE id = ?")
            .bind(&name)
            .bind(&color)
            .bind(id)
            .execute(&mut *tx)
            .await?;

        let Some(updated) = TagRow::load(&mut tx, id).await? else {
            return Ok(None);
        };

        tx.commit().await?;
        Ok(Some(Inserted::New(updated)))
    }
}

pub(crate) async fn get_or_create_tag(conn: &mut SqliteConnection, name: &Name) -> Result<i64> {
    // DO UPDATE (no-op) so RETURNING yields the existing row, keeping its original casing.
    sqlx::query_scalar(
        "INSERT INTO tag (name) VALUES (?)
         ON CONFLICT(name) DO UPDATE SET name = name
         RETURNING id",
    )
    .bind(name)
    .fetch_one(&mut *conn)
    .await
}

#[derive(Debug, FromRow)]
pub(crate) struct TagCount {
    pub id: i64,
    pub name: String,
    pub pages: i64,
    pub pending: i64,
}

pub(crate) async fn list_tag_counts(pool: &SqlitePool) -> Result<Vec<TagCount>, sqlx::Error> {
    sqlx::query_as(
        "SELECT t.id, t.name,
                (SELECT COUNT(*) FROM url_tag ut WHERE ut.tag_id = t.id)     AS pages,
                (SELECT COUNT(*) FROM pending_tag pt WHERE pt.tag_id = t.id) AS pending
         FROM tag t
         ORDER BY pages DESC, t.name COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await
}

// pub(crate) async fn delete_tag(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
//     let n = sqlx::query("DELETE FROM tag WHERE id = ?")
//         .bind(id)
//         .execute(pool)
//         .await?
//         .rows_affected();
//     Ok(n > 0)
// }

// /// Insert a tag. Returns `None` when the name already exists.
// pub(crate) async fn create_tag(
//     pool: &SqlitePool,
//     name: &str,
//     color: Option<&str>,
// ) -> Result<Option<TagRow>, sqlx::Error> {
//     sqlx::query_as(
//         "INSERT INTO tag (name, color) VALUES (?, ?)
//          ON CONFLICT(name) DO NOTHING
//          RETURNING id, name, color, created_at",
//     )
//     .bind(name)
//     .bind(color)
//     .fetch_optional(pool)
//     .await
// }
