use std::collections::HashMap;

use axum::{
    extract::{
        State,
        multipart::{Multipart, MultipartRejection},
    },
    http::StatusCode,
};
use futures::{StreamExt, stream};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::{
    blob_store::BlobStore,
    db::{
        helpers::{add_url_tags, get_or_create_url, to_hex},
        store::{insert_blob, prepare_blob},
    },
    downloader,
    dto::{ArchiveResponse, ArchiveUpload, AssetSummary, Dedup, ScreenshotFormat},
    error::{AppError, Result},
    extract::ApiJson,
    extractor::html,
    tag::clean_tags,
    url_canon::canonicalize_url,
};

/// Upper bound on sub-resources fetched per archive request.
const MAX_ASSETS: usize = 100;
/// Concurrent fetches. Kept modest so one archive cannot saturate the network.
const ASSET_CONCURRENCY: usize = 8;
/// Total fetched bytes per archive, on top of the per-resource limit.
const MAX_ASSET_TOTAL_BYTES: usize = 128 * 1024 * 1024;

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

/// A resource that has already been fetched and inserted into `content`.
struct PreparedAsset {
    url: String,
    kind: &'static str,
    status_code: Option<i64>,
    content_id: i64,
    /// Hex blake3 of the bytes, used to build rewritten `/api/content/{hash}` URLs.
    hash: String,
}

/// Parse, fetch and store the document's sub-resources.
///
/// Content rows are inserted before the capture transaction on purpose: network
/// I/O must not hold a write transaction. If the archive later fails, the
/// unreferenced rows have ref_count = 0 and are removed by the normal GC.
async fn collect_assets(
    pool: &SqlitePool,
    store: &BlobStore,
    client: &reqwest::Client,
    page_url: &url::Url,
    html: &[u8],
) -> Result<(Vec<PreparedAsset>, AssetSummary)> {
    let text = String::from_utf8_lossy(html);
    let found = html::extract_resources(page_url, &text);
    let found_count = found.len();
    let resources: Vec<_> = found.into_iter().take(MAX_ASSETS).collect();

    let mut summary = AssetSummary {
        skipped: found_count - resources.len(),
        ..AssetSummary::default()
    };
    let mut saved = Vec::with_capacity(resources.len());
    let mut total_bytes = 0usize;

    let mut fetches = stream::iter(resources)
        .map(|resource| {
            let client = client.clone();
            async move {
                let fetched =
                    downloader::fetch(&client, resource.url.clone(), downloader::MAX_ASSET_BYTES)
                        .await;
                (resource, fetched)
            }
        })
        .buffer_unordered(ASSET_CONCURRENCY);

    while let Some((resource, fetched)) = fetches.next().await {
        let fetched = match fetched {
            Ok(fetched) => fetched,
            Err(e) => {
                tracing::debug!(url = %resource.url, error = %e, "asset fetch failed");
                summary.failed += 1;
                continue;
            }
        };

        if total_bytes + fetched.bytes.len() > MAX_ASSET_TOTAL_BYTES {
            tracing::debug!(url = %resource.url, "asset budget exhausted");
            summary.skipped += 1;
            continue;
        }
        total_bytes += fetched.bytes.len();
        tracing::trace!(
            url = %resource.url,
            final_url = %fetched.final_url,
            bytes = fetched.bytes.len(),
            kind = resource.kind.as_str(),
            "asset fetched"
        );

        let blob = match prepare_blob(pool, store, fetched.bytes, &fetched.content_type).await {
            Ok(blob) => blob,
            Err(e) => {
                tracing::warn!(url = %resource.url, error = ?e, "asset storage failed");
                summary.failed += 1;
                continue;
            }
        };

        let content_id = {
            let mut conn = pool.acquire().await?;
            insert_blob(&mut conn, &blob).await?
        };

        summary.saved += 1;
        if blob.existed {
            summary.deduplicated += 1;
        }
        saved.push(PreparedAsset {
            url: resource.url.to_string(),
            kind: resource.kind.as_str(),
            status_code: Some(fetched.status as i64),
            content_id,
            hash: to_hex(&blob.hash),
        });
    }

    Ok((saved, summary))
}

async fn parse_multipart(mut mp: Multipart) -> Result<ArchiveUpload> {
    let mut up = ArchiveUpload::default();
    let mut shot: Option<Vec<u8>> = None;
    let mut fmt: Option<ScreenshotFormat> = None;

    while let Some(field) = mp.next_field().await? {
        let name = field.name().unwrap_or("");
        match name {
            "html" => up.html = field.bytes().await?.to_vec(),
            "url" => up.url = non_empty(field.text().await?),
            "title" => up.title = non_empty(field.text().await?),
            "tags" => {
                let t = field.text().await?;
                up.tags = serde_json::from_str(&t).map_err(|_| {
                    unprocessable("invalid_tags", "tags must be a JSON array of strings")
                })?;
            }
            "screenshot" => shot = Some(field.bytes().await?.to_vec()),
            "screenshot-format" => fmt = Some(parse_format(&field.text().await?)?),
            "pending_id" => {
                up.pending_id =
                    Some(field.text().await?.trim().parse().map_err(|_| {
                        AppError::BadRequest("pending_id must be an integer".into())
                    })?);
            }
            "metadata" => {
                let t = field.text().await?;
                up.metadata = Some(serde_json::from_str(&t).map_err(|e| {
                    unprocessable(
                        "invalid_metadata",
                        format!("metadata is not valid JSON: {e}"),
                    )
                })?);
            }
            "save-assets" | "save_assets" => {
                up.save_assets = Some(parse_bool(&field.text().await?)?);
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
    State(client): State<reqwest::Client>,
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

    // Split out sub-resource URLs before the HTML bytes are moved into the blob.
    let page_url = url::Url::parse(&canon.url).map_err(|e| AppError::Internal(Box::new(e)))?;
    let (assets, asset_summary) = if upload.save_assets.unwrap_or(true) {
        collect_assets(&pool, &store, &client, &page_url, &upload.html).await?
    } else {
        (Vec::new(), AssetSummary::default())
    };

    // Point every captured resource at its immutable content URL, then build the
    // document clients are served. The upload itself is kept byte-for-byte.
    let replacements: HashMap<String, String> = assets
        .iter()
        .map(|asset| (asset.url.clone(), format!("/api/content/{}", asset.hash)))
        .collect();
    let rewritten = if replacements.is_empty() {
        None
    } else {
        html::rewrite_resources(&upload.html, &page_url, &replacements)
    };

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
