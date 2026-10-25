use axum::{
    extract::{
        State,
        multipart::{Multipart, MultipartRejection},
    },
    http::StatusCode,
};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::{
    blob_store::BlobStore,
    db::{
        helpers::{add_url_tags, get_or_create_url},
        store::{insert_blob, prepare_blob},
    },
    dto::{ArchiveResponse, ArchiveUpload, Dedup, ScreenshotFormat},
    error::{AppError, Result},
    extract::ApiJson,
    tag::clean_tags,
    url_canon::canonicalize_url,
};

fn unprocessable(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::Unprocessable {
        code,
        message: message.into(),
    }
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn meta_str(meta: Option<&Value>, key: &str) -> Option<String> {
    meta?
        .get(key)?
        .as_str()
        .map(str::to_string)
        .and_then(non_empty)
}

fn parse_format(s: &str) -> Result<ScreenshotFormat> {
    match s.trim().to_ascii_lowercase().as_str() {
        "png" => Ok(ScreenshotFormat::Png),
        "jpeg" | "jpg" => Ok(ScreenshotFormat::Jpeg),
        _ => Err(unprocessable(
            "invalid_screenshot_format",
            "screenshot-format must be \"png\" or \"jpeg\"",
        )),
    }
}

async fn parse_multipart(mut mp: Multipart) -> Result<ArchiveUpload> {
    let mut up = ArchiveUpload::default();
    let mut shot: Option<Vec<u8>> = None;
    let mut fmt: Option<ScreenshotFormat> = None;

    while let Some(field) = mp.next_field().await? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "html" => up.html = field.bytes().await?.to_vec(),
            "screenshot" => shot = Some(field.bytes().await?.to_vec()),
            "screenshot-format" => fmt = Some(parse_format(&field.text().await?)?),
            "metadata" => {
                let t = field.text().await?;
                up.metadata = Some(serde_json::from_str(&t).map_err(|e| {
                    unprocessable(
                        "invalid_metadata",
                        format!("metadata is not valid JSON: {e}"),
                    )
                })?);
            }
            "url" => up.url = non_empty(field.text().await?),
            "title" => up.title = non_empty(field.text().await?),
            "tags" => {
                let t = field.text().await?;
                up.tags = serde_json::from_str(&t).map_err(|_| {
                    unprocessable("invalid_tags", "tags must be a JSON array of strings")
                })?;
            }
            "pending_id" => {
                up.pending_id =
                    Some(field.text().await?.trim().parse().map_err(|_| {
                        AppError::BadRequest("pending_id must be an integer".into())
                    })?);
            }
            _ => {} // ignore unknown fields
        }
    }

    if up.html.is_empty() {
        return Err(unprocessable(
            "missing_html",
            "the html field is required and must not be empty",
        ));
    }

    if let Some(bytes) = shot {
        let fmt = match fmt {
            Some(f) => f,
            None if ScreenshotFormat::Png.matches(&bytes) => ScreenshotFormat::Png,
            None if ScreenshotFormat::Jpeg.matches(&bytes) => ScreenshotFormat::Jpeg,
            None => {
                return Err(unprocessable(
                    "invalid_screenshot_format",
                    "cannot determine screenshot format",
                ));
            }
        };
        if !fmt.matches(&bytes) {
            return Err(unprocessable(
                "screenshot_format_mismatch",
                "screenshot bytes do not match screenshot-format",
            ));
        }
        up.screenshot = Some((bytes, fmt));
    }
    Ok(up)
}

// POST /api/archive
pub async fn archive(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, ApiJson<ArchiveResponse>)> {
    let upload = parse_multipart(multipart?).await?;

    // Pending row (optional) can supply url/title. A missing row is tolerated.
    let pending: Option<(String, Option<String>)> = match upload.pending_id {
        Some(pid) => {
            sqlx::query_as("SELECT url, title FROM pending WHERE id = ?")
                .bind(pid)
                .fetch_optional(&pool)
                .await?
        }
        None => None,
    };

    // Precedence: explicit field > metadata > pending row
    let raw_url = upload
        .url
        .clone()
        .or_else(|| meta_str(upload.metadata.as_ref(), "url"))
        .or_else(|| pending.as_ref().map(|p| p.0.clone()))
        .ok_or_else(|| {
            AppError::BadRequest("url is required (field, metadata.url, or pending_id)".into())
        })?;
    let canon = canonicalize_url(&raw_url)?;

    let title = upload
        .title
        .clone()
        .or_else(|| meta_str(upload.metadata.as_ref(), "title"))
        .or_else(|| {
            pending
                .as_ref()
                .and_then(|p| p.1.clone())
                .and_then(non_empty)
        });

    let tags = clean_tags(&upload.tags)?;
    let metadata_text = upload.metadata.as_ref().map(Value::to_string);

    // CPU-heavy work (hash + compress) happens before the write transaction.
    let html_blob = prepare_blob(&pool, &store, upload.html, "text/html").await?;
    let shot_blob = match upload.screenshot {
        Some((bytes, fmt)) => Some(prepare_blob(&pool, &store, bytes, fmt.mime()).await?),
        None => None,
    };

    let mut tx = pool.begin().await?;

    let url_id = get_or_create_url(&mut tx, &canon).await?;
    let html_id = insert_blob(&mut tx, &html_blob).await?;
    let shot_id = match &shot_blob {
        Some(b) => Some(insert_blob(&mut tx, b).await?),
        None => None,
    };

    let capture_id: i64 = sqlx::query_scalar(
        "INSERT INTO capture (url_id, content_id, screenshot_content_id, captured_at,
                              content_type, metadata, title)
         VALUES (?, ?, ?, unixepoch(), 'text/html', ?, ?)
         RETURNING id",
    )
    .bind(url_id)
    .bind(html_id)
    .bind(shot_id)
    .bind(&metadata_text)
    .bind(&title)
    .fetch_one(&mut *tx)
    .await?;

    let version: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM capture WHERE url_id = ?")
        .bind(url_id)
        .fetch_one(&mut *tx)
        .await?;

    // Tags are unioned; a re-archive can never remove tags.
    add_url_tags(&mut tx, url_id, &tags).await?;

    // Merge tags from the pending row (by id, or any queued row with the same URL) and dequeue.
    sqlx::query(
        "INSERT OR IGNORE INTO url_tag (url_id, tag_id)
         SELECT ?, pt.tag_id FROM pending_tag pt JOIN pending p ON p.id = pt.pending_id
         WHERE p.url = ? OR p.id = ?",
    )
    .bind(url_id)
    .bind(&canon.url)
    .bind(upload.pending_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM pending WHERE url = ? OR id = ?")
        .bind(&canon.url)
        .bind(upload.pending_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        ApiJson(ArchiveResponse {
            capture_id,
            page_id: url_id,
            version,
            deduplicated: Dedup {
                html: html_blob.existed,
                screenshot: shot_blob.as_ref().map(|b| b.existed),
            },
        }),
    ))
}
