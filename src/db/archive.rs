use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
};

use futures::{StreamExt, stream};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::{
    db::store::{insert_blob, prepare_blob},
    infra::{
        Blake3Hash, BlobStore, downloader,
        extractor::{css, html},
    },
    prelude::*,
};

/// Upper bound on sub-resources fetched per archive request (CSS resources included).
const MAX_ASSETS: usize = 100;
/// Concurrent fetches. Kept modest so one archive cannot saturate the network.
const ASSET_CONCURRENCY: usize = 8;
/// Total fetched bytes per archive, on top of the per-resource limit.
const MAX_ASSET_TOTAL_BYTES: usize = 128 * 1024 * 1024;
/// `@import` chains deeper than this are left unrewritten.
const MAX_CSS_DEPTH: u8 = 4;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssetSummary {
    /// Asset rows written for this capture.
    pub saved: usize,
    /// Saved assets whose bytes already existed in `content`.
    pub deduplicated: usize,
    /// Fetch or storage failures (the archive still succeeds).
    pub failed: usize,
    /// Resources that were deliberately not fetched (empty, over the cap, ...).
    pub skipped: usize,
}

/// A resource that has already been fetched and inserted into `content`.
pub(crate) struct PreparedAsset {
    pub url: String,
    pub kind: &'static str,
    pub status_code: Option<i64>,
    pub content_id: i64,
    /// Hex blake3 of the bytes, used to build rewritten `/api/content/{hash}` URLs.
    pub hash: Blake3Hash,
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

type StoreFuture<'b> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'b>>;

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
    #[expect(
        clippy::too_many_lines,
        reason = "storing an asset involves budget checks, rewriting, and persistence"
    )]
    fn store(
        &mut self,
        fetched: downloader::FetchedResource,
        url: url::Url,
        kind: html::AssetKind,
        depth: u8,
    ) -> StoreFuture<'_> {
        Box::pin(async move {
            let key = url.to_string();
            // A nested reference may point at a resource the HTML already listed.
            if self.saved.iter().any(|asset| asset.url == key) {
                return Ok(());
            }
            if self.saved.len() >= MAX_ASSETS {
                self.summary.skipped = self.summary.skipped.saturating_add(1);
                return Ok(());
            }
            if self.total_bytes.saturating_add(fetched.bytes.len()) > MAX_ASSET_TOTAL_BYTES {
                tracing::debug!(url = %key, "asset budget exhausted");
                self.summary.skipped = self.summary.skipped.saturating_add(1);
                return Ok(());
            }
            self.total_bytes = self.total_bytes.saturating_add(fetched.bytes.len());

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
            if kind == html::AssetKind::Stylesheet
                && depth < MAX_CSS_DEPTH
                && let Some(rewritten) = self.rewrite_css(&bytes, &final_url, depth).await?
            {
                bytes = rewritten.into_bytes();
            }

            let blob = match prepare_blob(self.pool, self.store, bytes, &content_type).await {
                Ok(blob) => blob,
                Err(e) => {
                    tracing::warn!(url = %key, error = ?e, "asset storage failed");
                    self.summary.failed = self.summary.failed.saturating_add(1);
                    return Ok(());
                }
            };

            let content_id = {
                let mut conn = self.pool.acquire().await?;
                insert_blob(&mut conn, &blob).await?
            };

            self.summary.saved = self.summary.saved.saturating_add(1);
            if blob.existed {
                self.summary.deduplicated = self.summary.deduplicated.saturating_add(1);
            }
            self.saved.push(PreparedAsset {
                url: key,
                kind: kind.as_str(),
                status_code: Some(i64::from(status)),
                content_id,
                hash: blob.hash,
            });
            Ok(())
        })
    }

    /// Fetch and store whatever a stylesheet references, then rewrite its URLs.
    #[expect(
        clippy::excessive_nesting,
        reason = "the loop mirrors the CSS reference structure"
    )]
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
                    replacements.insert(key, format!("/api/content/{}", asset.hash.to_hex()));
                }
                continue;
            }

            if self.saved.len() >= MAX_ASSETS {
                self.summary.skipped = self.summary.skipped.saturating_add(1);
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
                    self.store(
                        fetched,
                        resource.url,
                        resource.kind,
                        depth.saturating_add(1),
                    )
                    .await?;
                    if let Some(asset) = self.saved.iter().find(|asset| asset.url == key) {
                        replacements.insert(key, format!("/api/content/{}", asset.hash.to_hex()));
                    }
                }
                Err(e) => {
                    tracing::debug!(url = %key, error = %e, "css resource fetch failed");
                    self.summary.failed = self.summary.failed.saturating_add(1);
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
/// unreferenced rows have `ref_count` = 0 and are removed by the normal GC.
pub(crate) async fn collect_assets(
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
            skipped: found_count.saturating_sub(resources.len()),
            ..AssetSummary::default()
        },
        total_bytes: 0,
        seen: HashSet::new(),
        saved: Vec::with_capacity(resources.len()),
    };

    let mut in_flight = stream::iter(resources)
        .map(|resource| {
            let client = client.clone();
            async move {
                // TODO: Optionally use headers captured in meta
                let fetched =
                    downloader::fetch(&client, resource.url.clone(), downloader::MAX_ASSET_BYTES)
                        .await;
                (resource, fetched)
            }
        })
        .buffer_unordered(ASSET_CONCURRENCY);

    while let Some((resource, fetched)) = in_flight.next().await {
        match fetched {
            Ok(fetched) => {
                collector.seen.insert(resource.url.to_string());
                collector
                    .store(fetched, resource.url, resource.kind, 0)
                    .await?;
            }
            Err(e) => {
                tracing::debug!(url = %resource.url, error = %e, "asset fetch failed");
                collector.summary.failed = collector.summary.failed.saturating_add(1);
            }
        }
    }

    Ok((collector.saved, collector.summary))
}
