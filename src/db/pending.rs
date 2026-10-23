// db/pending.rs
use sqlx::{AssertSqlSafe, FromRow, QueryBuilder, Sqlite, SqlitePool};
use std::collections::HashMap;

use crate::dto::PendingDto;
use crate::error::{OptionExt, Result};
use crate::models::{PendingRow, TagRow};

const PENDING_COLS: &str = "id, url, title, status, attempts, last_error, created_at";

#[derive(FromRow)]
struct PendingTagRow {
    pending_id: i64,
    #[sqlx(flatten)]
    tag: TagRow, // id, name, color, created_at
}

pub async fn load_pending(pool: &SqlitePool, id: i64) -> Result<PendingDto> {
    // Aduit
    let row: PendingRow = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT {PENDING_COLS} FROM pending WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?
    .or_not_found("pending item")?;

    let mut dtos = hydrate_pending(pool, vec![row]).await?;
    Ok(dtos.remove(0))
}

/// Attach tags to many pending rows with a single extra query.
pub async fn hydrate_pending(pool: &SqlitePool, rows: Vec<PendingRow>) -> Result<Vec<PendingDto>> {
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

    let mut by_pending: HashMap<i64, Vec<TagRow>> = HashMap::new();
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

/// For GET /api/pending (keyset pagination on id, oldest first)
pub async fn list_pending(
    pool: &SqlitePool,
    status: Option<crate::models::PendingStatus>,
    after_id: Option<i64>,
    limit: u32,
) -> Result<crate::dto::Page<PendingDto>> {
    let limit = limit.clamp(1, 200) as i64;

    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new(format!("SELECT {PENDING_COLS} FROM pending WHERE id > "));
    qb.push_bind(after_id.unwrap_or(0));
    if let Some(s) = status {
        qb.push(" AND status = ").push_bind(s);
    }
    qb.push(" ORDER BY id LIMIT ").push_bind(limit + 1); // fetch one extra to detect "has more"

    let mut rows: Vec<PendingRow> = qb.build_query_as().fetch_all(pool).await?;
    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| rows.last().map(|r| r.id.to_string()))
        .flatten();

    Ok(crate::dto::Page {
        items: hydrate_pending(pool, rows).await?,
        next_cursor,
    })
}
