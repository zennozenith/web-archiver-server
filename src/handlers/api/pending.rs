use std::borrow::Cow;

use axum::{extract::State, http::StatusCode};
use sqlx::SqlitePool;
use url::{Url, form_urlencoded};

use crate::{
    db,
    dto::{CreatePending, PendingDto},
    error::{AppError, Result},
    extract::{ApiJson, JsonOrForm},
};

pub async fn create_pending(
    State(pool): State<SqlitePool>,
    JsonOrForm(req): JsonOrForm<CreatePending>,
) -> Result<(StatusCode, ApiJson<PendingDto>)> {
    let canon = canonicalize_url(&req.url)?; // returns AppError::InvalidUrl on failure

    let mut tx = pool.begin().await?;

    // Insert; if the URL is already queued, return the existing id.
    let inserted: Option<i64> = sqlx::query_scalar(
        "INSERT INTO pending (url, title) VALUES (?, ?)
         ON CONFLICT(url) DO NOTHING
         RETURNING id",
    )
    .bind(&canon.url)
    .bind(&req.title)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(pending_id) = inserted else {
        let existing_id: i64 = sqlx::query_scalar("SELECT id FROM pending WHERE url = ?")
            .bind(&canon.url)
            .fetch_one(&mut *tx)
            .await?;
        return Err(AppError::DuplicatePending { existing_id });
    };

    for name in req.tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
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

    let dto = db::pending::load_pending(&pool, pending_id).await?; // joins tags
    Ok((StatusCode::CREATED, ApiJson(dto)))
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
