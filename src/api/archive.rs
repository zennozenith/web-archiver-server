use std::collections::HashMap;

use axum::extract::{Multipart, State, multipart::MultipartRejection};
use http::StatusCode;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::{
    blob_store::BlobStore,
    db::{
        archive::collect_assets,
        pages::{add_page_tags, get_or_create_url},
        pending::pending_url_title_optional,
        store::{insert_blob, prepare_blob},
    },
    dto::{ArchiveResponse, ArchiveUpload, AssetSummary, Dedup, ScreenshotFormat},
    error::{AppError, Result},
    extract::ApiJson,
    extractor::html,
    tag,
    url_canon::canonicalize_url,
};

fn unprocessable(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::Unprocessable {
        code,
        message: message.into(),
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

fn meta_str(meta: Option<&Value>, key: &str) -> Option<String> {
    meta?.get(key)?.as_str().and_then(non_empty)
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

fn parse_bool(s: &str) -> Result<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(unprocessable(
            "invalid_save_assets",
            "save-assets must be true or false",
        )),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one match arm per accepted multipart field"
)]
async fn parse_multipart(mut mp: Multipart) -> Result<ArchiveUpload> {
    let mut upload = ArchiveUpload::default();
    let mut shot: Option<Vec<u8>> = None;
    let mut shot_fmt: Option<ScreenshotFormat> = None;

    while let Some(field) = mp.next_field().await? {
        let name = field.name().unwrap_or("");
        match name {
            "html" => upload.html = field.bytes().await?.to_vec(),
            "url" => upload.url = non_empty(&field.text().await?),
            "title" => upload.title = non_empty(&field.text().await?),
            "tags" => {
                let t = field.text().await?;
                upload.tags = serde_json::from_str(&t).map_err(|_| {
                    unprocessable("invalid_tags", "tags must be a JSON array of strings")
                })?;
            }
            "screenshot" => shot = Some(field.bytes().await?.to_vec()),
            "screenshot-format" => shot_fmt = Some(parse_format(&field.text().await?)?),
            "pending_id" => {
                upload.pending_id =
                    Some(field.text().await?.trim().parse().map_err(|_| {
                        AppError::BadRequest("pending_id must be an integer".into())
                    })?);
            }
            "metadata" => {
                let t = field.text().await?;
                upload.metadata = Some(serde_json::from_str(&t).map_err(|e| {
                    unprocessable(
                        "invalid_metadata",
                        format!("metadata is not valid JSON: {e}"),
                    )
                })?);
            }
            "save-assets" | "save_assets" => {
                upload.save_assets = Some(parse_bool(&field.text().await?)?);
            }
            _ => {} // ignore unknown fields
        }
    }

    if upload.html.is_empty() {
        return Err(unprocessable(
            "missing_html",
            "the html field is required and must not be empty",
        ));
    }

    if let Some(bytes) = shot {
        let fmt = match shot_fmt {
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
        upload.screenshot = Some((bytes, fmt));
    }
    Ok(upload)
}

// POST /api/archive
#[expect(
    clippy::too_many_lines,
    reason = "the archive transaction is intentionally kept in one readable flow"
)]
pub(super) async fn archive(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    State(client): State<reqwest::Client>,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, ApiJson<ArchiveResponse>)> {
    let upload = parse_multipart(multipart?).await?;

    // Pending row (optional) can supply url/title. A missing row is tolerated.
    let pending: Option<(String, Option<String>)> = match upload.pending_id {
        Some(pid) => pending_url_title_optional(&pool, pid).await?,
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
                .and_then(|s| non_empty(&s))
        });

    let tags = tag::clean_names(&upload.tags)?;
    let metadata_text = upload.metadata.as_ref().map(Value::to_string);

    // Split out sub-resource URLs before the HTML bytes are moved into the blob.
    let page_url = url::Url::parse(&canon.url).map_err(|e| AppError::Internal(Box::new(e)))?;
    let (assets, asset_summary) = if upload.save_assets.unwrap_or(true) {
        collect_assets(&pool, &store, &client, &page_url, &upload.html).await?
    } else {
        (Vec::new(), AssetSummary::default())
    };

    // Point every captured resource at its immutable content URL, then build the
    // document clients are served. The upload itself is kept byte-for-byte.
    // Even without captured assets the HTML is rewritten so relative `<a href>`
    // links can be resolved against the original page URL.
    let replacements: HashMap<String, String> = assets
        .iter()
        .map(|asset| (asset.url.clone(), format!("/api/content/{}", asset.hash)))
        .collect();
    let rewritten = html::rewrite_resources(&upload.html, &page_url, &replacements);

    // CPU-heavy work (hash + compress) happens before the write transaction.
    let original_blob = prepare_blob(&pool, &store, upload.html, "text/html").await?;
    let served_blob = match rewritten {
        Some(bytes) => Some(prepare_blob(&pool, &store, bytes, "text/html").await?),
        None => None,
    };
    let shot_blob = match upload.screenshot {
        Some((bytes, fmt)) => Some(prepare_blob(&pool, &store, bytes, fmt.mime()).await?),
        None => None,
    };

    let mut tx = pool.begin().await?;

    let url_id = get_or_create_url(&mut tx, &canon).await?;
    let original_id = insert_blob(&mut tx, &original_blob).await?;
    let html_id = match &served_blob {
        Some(b) => insert_blob(&mut tx, b).await?,
        None => original_id,
    };
    let shot_id = match &shot_blob {
        Some(b) => Some(insert_blob(&mut tx, b).await?),
        None => None,
    };

    let capture_id: i64 = sqlx::query_scalar(
        "INSERT INTO capture (url_id, content_id, original_content_id, screenshot_content_id,
                              captured_at, content_type, metadata, title)
         VALUES (?, ?, ?, ?, unixepoch(), 'text/html', ?, ?)
         RETURNING id",
    )
    .bind(url_id)
    .bind(html_id)
    .bind(original_id)
    .bind(shot_id)
    .bind(&metadata_text)
    .bind(&title)
    .fetch_one(&mut *tx)
    .await?;

    for asset in &assets {
        sqlx::query(
            "INSERT OR IGNORE INTO capture_asset (capture_id, content_id, url, kind, status_code)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(capture_id)
        .bind(asset.content_id)
        .bind(&asset.url)
        .bind(asset.kind)
        .bind(asset.status_code)
        .execute(&mut *tx)
        .await?;
    }

    let version: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM capture WHERE url_id = ?")
        .bind(url_id)
        .fetch_one(&mut *tx)
        .await?;

    // Tags are unioned; a re-archive can never remove tags.
    add_page_tags(&mut tx, url_id, &tags).await?;

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
                html: served_blob
                    .as_ref()
                    .map_or(original_blob.existed, |b| b.existed),
                original_html: original_blob.existed,
                screenshot: shot_blob.as_ref().map(|b| b.existed),
            },
            assets: asset_summary,
        }),
    ))
}
