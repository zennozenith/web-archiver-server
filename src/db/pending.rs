// db/pending.rs
use sqlx::{AssertSqlSafe, FromRow, QueryBuilder, Sqlite, SqlitePool};
use std::borrow::Cow;
use std::collections::HashMap;
use url::{Url, form_urlencoded};

use crate::dto::{self, PendingDto};
use crate::error::{AppError, OptionExt, Result};
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

pub async fn create_pending(pool: &SqlitePool, data: dto::CreatePending) -> Result<PendingDto> {
    let canon = canonicalize_url(&data.url)?; // returns AppError::InvalidUrl on failure

    let mut tx = pool.begin().await?;

    // Insert; if the URL is already queued, return the existing id.
    let inserted: Option<i64> = sqlx::query_scalar(
        "INSERT INTO pending (url, title) VALUES (?, ?)
         ON CONFLICT(url) DO NOTHING
         RETURNING id",
    )
    .bind(&canon.url)
    .bind(&data.title)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(pending_id) = inserted else {
        let existing_id: i64 = sqlx::query_scalar("SELECT id FROM pending WHERE url = ?")
            .bind(&canon.url)
            .fetch_one(&mut *tx)
            .await?;
        return Err(AppError::DuplicatePending { existing_id });
    };

    for name in data.tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        // get-or-create tag (NOCASE unique), then link
        let tag_id: i64 = sqlx::query_scalar(
            "INSERT INTO tag (name) VALUES (?)
             ON CONFLICT(name) DO UPDATE SET name = name
             RETURNING id",
        )
        .bind(name)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query("INSERT OR IGNORE INTO pending_tag (pending_id, tag_id) VALUES (?, ?)")
            .bind(pending_id)
            .bind(tag_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    load_pending(&pool, pending_id).await // joins tags
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }
}

pub struct CanonicalUrl {
    pub url: String,
    pub scheme: Scheme,
    pub host: String,
    pub path: String,
    pub query: Option<String>,
}

pub fn canonicalize_url(raw_url: &str) -> Result<CanonicalUrl> {
    let mut parsed = Url::parse(raw_url.trim()).map_err(|e| AppError::InvalidUrl(e.to_string()))?;

    // Remove fragment/hash (not part of server-side resource identification)
    parsed.set_fragment(None);

    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);

    // Normalize and sort query parameters alphabetically
    if let Some(query) = parsed.query() {
        let mut pairs: Vec<(Cow<str>, Cow<str>)> =
            form_urlencoded::parse(query.as_bytes()).collect();
        // Sort by key, then by value
        pairs.sort_by(|a, b| a.0.cmp(&b.0));

        // Re-serialization rewrites encoding.
        // form_urlencoded::Serializer turns %20 into + and ?flag into ?flag=.
        // That’s fine for deduplication, but it means the stored URL may not be
        // byte-identical to what you actually fetch. If you fetch using the
        // canonical URL, this is harmless. If you fetch the user’s original,
        // keep the original in capture or metadata
        let sorted_query = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();

        if sorted_query.is_empty() {
            parsed.set_query(None);
        } else {
            parsed.set_query(Some(&sorted_query));
        }
    }

    // Extract components matching your SQLite schema
    let scheme = match parsed.scheme() {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        _ => {
            return Err(AppError::InvalidUrl(
                "only http and https are supported".to_string(),
            ));
        }
    };

    let host = parsed.host_str().unwrap_or("").to_string();

    let path = parsed.path().to_string();
    let query = parsed.query().map(|q| q.to_string());

    // The final normalized string representation (automatically strips default ports)
    let canonical_url = parsed.to_string();

    Ok(CanonicalUrl {
        url: canonical_url,
        scheme,
        host,
        path,
        query,
    })
}

// fn normalize_url(raw: &str) -> Result<String> {
//     // use the `url` crate: parse, require http/https, strip fragment, lowercase host
//     let mut u = Url::parse(raw.trim()).map_err(|e| AppError::InvalidUrl(e.to_string()))?;
//     if !matches!(u.scheme(), "http" | "https") {
//         return Err(AppError::InvalidUrl(
//             "only http and https are supported".into(),
//         ));
//     }
//     u.set_fragment(None);
//     Ok(u.into())
// }
