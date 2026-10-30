// db/pending.rs
use sqlx::{AssertSqlSafe, FromRow, QueryBuilder, Sqlite, SqliteConnection, SqlitePool};
use std::collections::HashMap;

use super::helpers::replace_pending_tags;

use crate::dto::PendingDto;
use crate::models::PendingRow;
use crate::models::{self, PendingStatus};
use crate::tag::CleanTag;
use crate::url_canon::CanonicalUrl;

const PENDING_COLS: &str = "id, url, title, status, attempts, last_error, created_at";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Inserted {
    New(i64),
    Existing(i64),
}

/// Shared by create + bulk. Tags are only attached when the row is new.
pub(crate) async fn insert_pending(
    conn: &mut SqliteConnection,
    url: &CanonicalUrl,
    title: Option<&str>,
    tags: &[CleanTag],
) -> Result<Inserted, sqlx::Error> {
    let title = title.map(str::trim).filter(|t| !t.is_empty());

    let new_id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO pending (url, title) VALUES (?, ?)
         ON CONFLICT(url) DO NOTHING RETURNING id",
    )
    .bind(&url.url)
    .bind(title)
    .fetch_optional(&mut *conn)
    .await?;

    let Some(id) = new_id else {
        let existing: i64 = sqlx::query_scalar("SELECT id FROM pending WHERE url = ?")
            .bind(&url.url)
            .fetch_one(&mut *conn)
            .await?;

        // TODO: Update tags
        return Ok(Inserted::Existing(existing));
    };

    replace_pending_tags(&mut *conn, id, tags).await?;
    Ok(Inserted::New(id))
}

/// Call once at startup: anything left `in_progress` belongs to a dead worker.
#[expect(
    dead_code,
    reason = "startup recovery hook for deployments with external workers"
)]
pub(crate) async fn reset_in_progress(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    Ok(
        sqlx::query("UPDATE pending SET status = 'queued' WHERE status = 'in_progress'")
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

/// TODO: docs
pub(crate) async fn pending_status(
    tx: &mut SqliteConnection,
    id: i64,
) -> Result<Option<PendingStatus>, sqlx::Error> {
    sqlx::query_scalar("SELECT status FROM pending WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
}

/// TODO: docs
pub(crate) async fn update_pending(
    tx: &mut SqliteConnection,
    id: i64,
    title: Option<&str>,
    status: Option<PendingStatus>,
    tags: Option<&Vec<CleanTag>>,
) -> Result<(), sqlx::Error> {
    if let Some(title) = title {
        let title = Some(title.trim()).filter(|t| !t.is_empty()); // "" clears the title
        sqlx::query("UPDATE pending SET title = ? WHERE id = ?")
            .bind(title)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(status) = status {
        sqlx::query(
            "UPDATE pending SET status = ?,
                    last_error = CASE WHEN ? = 'queued' THEN NULL ELSE last_error END
             WHERE id = ?",
        )
        .bind(status)
        .bind(status)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(tags) = tags {
        replace_pending_tags(&mut *tx, id, tags).await?;
    }

    Ok(())
}

/// TODO: docs
pub(crate) async fn claim_pending(pool: &SqlitePool) -> Result<Option<i64>, sqlx::Error> {
    let id: Option<i64> = sqlx::query_scalar(
        "UPDATE pending SET status = 'in_progress'
         WHERE id = (SELECT id FROM pending WHERE status = 'queued'
                     ORDER BY created_at, id LIMIT 1)
         RETURNING id",
    )
    .fetch_optional(pool)
    .await?;

    Ok(id)
}

/// TODO: docs
pub(crate) async fn retry_pending(pool: &SqlitePool, id: i64) -> Result<u64, sqlx::Error> {
    let n = sqlx::query(
        "UPDATE pending SET status = 'queued', last_error = NULL WHERE id = ? AND status = 'failed'",
    )
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(n)
}

/// TODO: docs
pub(crate) async fn fail_pending(
    pool: &SqlitePool,
    id: i64,
    error: &str,
) -> Result<u64, sqlx::Error> {
    let n = sqlx::query(
        "UPDATE pending SET status = 'failed', attempts = attempts + 1, last_error = ?
         WHERE id = ? AND status = 'in_progress'",
    )
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(n)
}

/// TODO: docs
pub(crate) async fn delete_pending(pool: &SqlitePool, id: i64) -> Result<u64, sqlx::Error> {
    let n = sqlx::query("DELETE FROM pending WHERE id = ? AND status != 'in_progress'")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();

    Ok(n)
}

#[derive(FromRow)]
struct PendingTagRow {
    pending_id: i64,
    #[sqlx(flatten)]
    tag: models::TagRow, // id, name, color, created_at
}

pub(crate) async fn load_pending(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<PendingDto>, sqlx::Error> {
    // Aduit
    if let Some(row) = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT {PENDING_COLS} FROM pending WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?
    {
        let mut dtos: Vec<PendingDto> = hydrate_pending(pool, vec![row]).await?;
        // TODO: dtos will have atleast one element here, else remove will panic
        return Ok(Some(dtos.remove(0)));
    }

    Ok(None)
}

/// Attach tags to many pending rows with a single extra query.
pub(crate) async fn hydrate_pending(
    pool: &SqlitePool,
    rows: Vec<PendingRow>,
) -> Result<Vec<PendingDto>, sqlx::Error> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT pt.pending_id, t.id, t.name, t.color, t.created_at
         FROM pending_tag pt
         JOIN tag t ON t.id = pt.tag_id
         WHERE pt.pending_id IN (",
    );
    let mut sep = qb.separated(", ");
    for r in &rows {
        sep.push_bind(r.id);
    }
    sep.push_unseparated(") ORDER BY t.name COLLATE NOCASE");

    let tag_rows: Vec<PendingTagRow> = qb.build_query_as().fetch_all(pool).await?;

    let mut by_pending: HashMap<i64, Vec<models::TagRow>> = HashMap::new();
    for t in tag_rows {
        by_pending.entry(t.pending_id).or_default().push(t.tag);
    }

    Ok(rows
        .into_iter()
        .map(|r| PendingDto {
            tags: by_pending.remove(&r.id).unwrap_or_default(),
            id: r.id,
            url: r.url,
            title: r.title,
            status: r.status,
            attempts: r.attempts,
            last_error: r.last_error,
            created_at: r.created_at,
        })
        .collect())
}

// TODO: order by ascending/descending
/// For GET /api/pending (keyset pagination on id, new first)
pub(crate) async fn list_pending(
    pool: &SqlitePool,
    status: Option<crate::models::PendingStatus>,
    after_id: Option<i64>,
    limit: u32,
) -> Result<crate::dto::Page<PendingDto>, sqlx::Error> {
    let limit = i64::from(limit.clamp(1, 200));

    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new(format!("SELECT {PENDING_COLS} FROM pending WHERE id > "));
    qb.push_bind(after_id.unwrap_or(0));
    if let Some(s) = status {
        qb.push(" AND status = ").push_bind(s);
    }
    qb.push(" ORDER BY id DESC LIMIT ")
        .push_bind(limit.saturating_add(1)); // fetch one extra to detect "has more"

    let mut rows: Vec<PendingRow> = qb.build_query_as().fetch_all(pool).await?;
    let has_more = i64::try_from(rows.len()).map_or(true, |len| len > limit);
    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    let next_cursor = has_more
        .then(|| rows.last().map(|r| r.id.to_string()))
        .flatten();

    Ok(crate::dto::Page {
        items: hydrate_pending(pool, rows).await?,
        next_cursor,
    })
}

// pub async fn delete_pending(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
//     let result = sqlx::query("DELETE FROM pending WHERE id = ?")
//         .bind(id)
//         .execute(pool)
//         .await?;

//     Ok(result.rows_affected() > 0)
// }
