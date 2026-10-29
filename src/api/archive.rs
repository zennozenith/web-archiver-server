use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

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
    extractor::{css, html},
    tag::clean_tags,
    url_canon::canonicalize_url,
};

/// Upper bound on sub-resources fetched per archive request (CSS resources included).
const MAX_ASSETS: usize = 100;
/// Concurrent fetches. Kept modest so one archive cannot saturate the network.
const ASSET_CONCURRENCY: usize = 8;
/// Total fetched bytes per archive, on top of the per-resource limit.
const MAX_ASSET_TOTAL_BYTES: usize = 128 * 1024 * 1024;
/// `@import` chains deeper than this are left unrewritten.
const MAX_CSS_DEPTH: u8 = 4;

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

/// Classify by what the server actually returned, falling back to the URL
/// extension and then to the hint from the referring HTML/CSS.
fn refine_kind(suggested: html::AssetKind, content_type: &str, url: &url::Url) -> html::AssetKind {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    if ct == "text/css" {
        html::AssetKind::Stylesheet
    } else if ct.starts_with("font/") || ct.contains("font") {
        html::AssetKind::Font
    } else if ct.starts_with("image/") {
        html::AssetKind::Image
    } else if ct.starts_with("audio/") || ct.starts_with("video/") {
        html::AssetKind::Media
    } else {
        kind_from_extension(url).unwrap_or(suggested)
    }
}

fn kind_from_extension(url: &url::Url) -> Option<html::AssetKind> {
    let path = url.path().to_ascii_lowercase();
    let ext = path.rsplit('.').next()?;
    Some(match ext {
        "css" => html::AssetKind::Stylesheet,
        "woff" | "woff2" | "ttf" | "otf" | "eot" => html::AssetKind::Font,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "ico" | "bmp" => {
            html::AssetKind::Image
        }
        "mp4" | "webm" | "ogg" | "mp3" | "wav" | "m4a" | "m4v" | "mov" => html::AssetKind::Media,
        "js" | "mjs" => html::AssetKind::Script,
        _ => return None,
    })
}

/// State shared by the top-level fetch loop and the CSS recursion.
struct AssetCollector<'a> {
    pool: &'a SqlitePool,
    store: &'a BlobStore,
    client: &'a reqwest::Client,
    summary: AssetSummary,
    total_bytes: usize,
    seen: HashSet<String>,
    saved: Vec<PreparedAsset>,
}

impl AssetCollector<'_> {
    /// Store one fetched resource, rewriting stylesheets first.
    ///
    /// Boxed because stylesheets recurse back into this method for their own
    /// resources (`@import`, fonts, images) and async functions cannot be
    /// recursive otherwise.
    fn store<'b>(
        &'b mut self,
        fetched: downloader::FetchedResource,
        url: url::Url,
        kind: html::AssetKind,
        depth: u8,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'b>> {
        Box::pin(async move {
            let key = url.to_string();
            // A nested reference may point at a resource the HTML already listed.
            if self.saved.iter().any(|asset| asset.url == key) {
                return Ok(());
            }
            if self.saved.len() >= MAX_ASSETS {
                self.summary.skipped += 1;
                return Ok(());
            }
            if self.total_bytes + fetched.bytes.len() > MAX_ASSET_TOTAL_BYTES {
                tracing::debug!(url = %key, "asset budget exhausted");
                self.summary.skipped += 1;
                return Ok(());
            }
            self.total_bytes += fetched.bytes.len();

            let downloader::FetchedResource {
                final_url,
                status,
                content_type,
                bytes,
            } = fetched;
            let kind = refine_kind(kind, &content_type, &url);
            tracing::trace!(
                url = %key,
                final_url = %final_url,
                bytes = bytes.len(),
                kind = kind.as_str(),
                "asset fetched"
            );

            let mut bytes = bytes;
            if kind == html::AssetKind::Stylesheet && depth < MAX_CSS_DEPTH {
                if let Some(rewritten) = self.rewrite_css(&bytes, &final_url, depth).await? {
                    bytes = rewritten.into_bytes();
                }
            }

            let blob = match prepare_blob(self.pool, self.store, bytes, &content_type).await {
                Ok(blob) => blob,
                Err(e) => {
                    tracing::warn!(url = %key, error = ?e, "asset storage failed");
                    self.summary.failed += 1;
                    return Ok(());
                }
            };

            let content_id = {
                let mut conn = self.pool.acquire().await?;
                insert_blob(&mut conn, &blob).await?
            };

            self.summary.saved += 1;
            if blob.existed {
                self.summary.deduplicated += 1;
            }
            self.saved.push(PreparedAsset {
                url: key,
                kind: kind.as_str(),
                status_code: Some(status as i64),
                content_id,
                hash: to_hex(&blob.hash),
            });
            Ok(())
        })
    }

    /// Fetch and store whatever a stylesheet references, then rewrite its URLs.
    async fn rewrite_css(
        &mut self,
        bytes: &[u8],
        base: &url::Url,
        depth: u8,
    ) -> Result<Option<String>> {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return Ok(None);
        };
        let found = css::extract_resources(text, base);
        if found.is_empty() {
            return Ok(None);
        }

        let mut replacements = HashMap::new();
        for resource in found {
            let key = resource.url.to_string();

            if !self.seen.insert(key.clone()) {
                if let Some(asset) = self.saved.iter().find(|asset| asset.url == key) {
                    replacements.insert(key, format!("/api/content/{}", asset.hash));
                }
                continue;
            }

            if self.saved.len() >= MAX_ASSETS {
                self.summary.skipped += 1;
                continue;
            }

            match downloader::fetch(
                self.client,
                resource.url.clone(),
                downloader::MAX_ASSET_BYTES,
            )
            .await
            {
                Ok(fetched) => {
                    self.store(fetched, resource.url, resource.kind, depth + 1)
                        .await?;
                    if let Some(asset) = self.saved.iter().find(|asset| asset.url == key) {
                        replacements.insert(key, format!("/api/content/{}", asset.hash));
                    }
                }
                Err(e) => {
                    tracing::debug!(url = %key, error = %e, "css resource fetch failed");
                    self.summary.failed += 1;
                }
            }
        }

        if replacements.is_empty() {
            return Ok(None);
        }
        Ok(css::rewrite_resources(text, base, &replacements))
    }
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

    let mut collector = AssetCollector {
        pool,
        store,
        client,
        summary: AssetSummary {
            skipped: found_count - resources.len(),
            ..AssetSummary::default()
        },
        total_bytes: 0,
        seen: HashSet::new(),
        saved: Vec::with_capacity(resources.len()),
    };

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
        match fetched {
            Ok(fetched) => {
                collector.seen.insert(resource.url.to_string());
                collector
                    .store(fetched, resource.url, resource.kind, 0)
                    .await?;
            }
            Err(e) => {
                tracing::debug!(url = %resource.url, error = %e, "asset fetch failed");
                collector.summary.failed += 1;
            }
        }
    }

    Ok((collector.saved, collector.summary))
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
