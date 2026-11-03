use std::collections::HashMap;

use axum::extract::{Multipart, Path, State, multipart::MultipartRejection};
use http::StatusCode;
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::{
    blob_store::BlobStore,
    db::{
        archive::{AssetSummary, collect_assets},
        page::{PageId, PageRecord},
        pending::{PendingId, PendingRecord},
        store::{insert_blob, prepare_blob},
    },
    error::{AppError, OptionExt, Result},
    extract::ApiJson,
    extractor::html,
    helper::{NonEmptyString, parse_bool},
    image::{Image, ImageFormat},
};

// ── Requests ────────────────────────────────────────────────────────────────

/// Parsed from the multipart fields of POST /api/archive
#[derive(Debug, Default)]
struct ArchiveUpload {
    pub title: Option<NonEmptyString>,
    pub html: Vec<u8>,
    pub screenshot: Option<Image>,
    pub metadata: Option<serde_json::Value>,
    pub pending_id: Option<PendingId>,
    /// `save-assets=false` skips fetching sub-resources. Defaults to true.
    pub save_assets: Option<bool>,
}

// ── Responses ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArchiveResponse {
    pub capture_id: i64,
    pub page_id: PageId,
    pub version: i64,
    pub deduplicated: Dedup,
    pub assets: AssetSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Dedup {
    /// The served (rewritten) HTML blob already existed.
    pub html: bool,
    /// The exact uploaded HTML blob already existed.
    pub original_html: bool,
    pub screenshot: Option<bool>,
}

fn meta_str(meta: Option<&Value>, key: &str) -> Option<String> {
    meta?.get(key)?.as_str().and_then(NonEmptyString::non_empty)
}

async fn parse_multipart(mut mp: Multipart) -> Result<ArchiveUpload> {
    let mut upload = ArchiveUpload::default();
    let mut screenshot: Option<Vec<u8>> = None;

    while let Some(field) = mp.next_field().await? {
        let name = field.name().unwrap_or("");
        match name {
            "html" => upload.html = field.bytes().await?.to_vec(),
            "title" => upload.title = Some(NonEmptyString::trimmed(&field.text().await?)?),
            "screenshot" => screenshot = Some(field.bytes().await?.to_vec()),
            "pending_id" => {
                upload.pending_id = Some(
                    field
                        .text()
                        .await?
                        .trim()
                        .parse::<i64>()
                        .map_err(|_| AppError::BadRequest("pending_id must be an integer".into()))?
                        .into(),
                );
            }
            "metadata" => {
                let t = field.text().await?;
                upload.metadata = Some(serde_json::from_str(&t).map_err(|e| {
                    AppError::invalid(
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
        return Err(AppError::invalid(
            "missing_html",
            "the html field is required and must not be empty",
        ));
    }

    if let Some(bytes) = screenshot {
        let format = ImageFormat::detect(&bytes).ok_or_else(|| {
            AppError::invalid("invalid_image_format", "cannot determine screenshot format")
        })?;
        upload.screenshot = Some(Image::new(format, bytes)?);
    }

    Ok(upload)
}

// POST /page/{id}/capture
#[expect(
    clippy::too_many_lines,
    reason = "the archive transaction is intentionally kept in one readable flow"
)]
pub(super) async fn archive(
    State(pool): State<SqlitePool>,
    State(store): State<BlobStore>,
    State(client): State<reqwest::Client>,
    Path(id): Path<PageId>,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, ApiJson<ArchiveResponse>)> {
    let upload = parse_multipart(multipart?).await?;

    let mut tx = pool.begin().await?;

    let page = PageRecord::load(&mut *tx, id).await?.or_not_found("page")?;

    // Pending row (optional) can supply url/title. A missing row is tolerated.
    let pending = match upload.pending_id {
        Some(pid) => PendingRecord::load(&mut *tx, pid).await?,
        None => None,
    };

    let title = upload.title.clone().or_else(|| {
        meta_str(upload.metadata.as_ref(), "title")
            .map(NonEmptyString::trimmed)
            .transpose()
            .ok()
            .unwrap_or_default()
    });

    let metadata_text = upload.metadata.as_ref().map(Value::to_string);

    let page_url = url::Url::parse(&page.url).map_err(|e| AppError::Internal(Box::new(e)))?;

    // Split out sub-resource URLs before the HTML bytes are moved into the blob.
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
        .map(|asset| {
            (
                asset.url.clone(),
                format!("/api/content/{}", asset.hash.to_hex()),
            )
        })
        .collect();
    let rewritten = html::rewrite_resources(&upload.html, &page_url, &replacements);

    // CPU-heavy work (hash + compress) happens before the write transaction.
    let original_blob = prepare_blob(&pool, &store, upload.html, "text/html").await?;
    let served_blob = match rewritten {
        Some(bytes) => Some(prepare_blob(&pool, &store, bytes, "text/html").await?),
        None => None,
    };
    let shot_blob = match upload.screenshot {
        Some(image) => {
            let format = image.format();
            let data = image.into_data();
            Some(prepare_blob(&pool, &store, data, format.mime_type()).await?)
        }
        None => None,
    };

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
        "INSERT INTO capture (page_id, content_id, original_content_id, screenshot_content_id,
                              captured_at, content_type, metadata, title)
         VALUES (?, ?, ?, ?, unixepoch(), 'text/html', ?, ?)
         RETURNING id",
    )
    .bind(id)
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

    let version: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM capture WHERE page_id = ?")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

    if let Some(p) = pending {
        // Merge tags from the pending row (by id, or any queued row with the same URL) and dequeue.
        sqlx::query(
            "INSERT OR IGNORE INTO page_tag (page_id, tag_id)
         SELECT ?, pt.tag_id FROM pending_tag pt JOIN pending p ON p.id = pt.pending_id
         WHERE p.url = ? OR p.id = ?",
        )
        .bind(id)
        .bind(&page.url)
        .bind(p.id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM pending WHERE url = ? OR id = ?")
            .bind(&page.url)
            .bind(upload.pending_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        ApiJson(ArchiveResponse {
            capture_id,
            page_id: id,
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
