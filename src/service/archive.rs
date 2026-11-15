//! Capturing a page: rewrite its HTML, store the blobs, write the capture row.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

use crate::{
    app::AppState,
    db::{
        archive::{AssetSummary, collect_assets, insert_assets},
        blob::{insert_blob, prepare_blob},
        capture::NewCapture,
        page::{PageId, PageRecord},
        pending::{PendingId, PendingRow},
    },
    error::{AppError, OptionNotFound as _, Resource, Result},
    identity::{AuthUser, Permission},
    infra::{Image, extractor::html},
    util::{NonEmptyString, time::now_ms},
};

/// Parsed multipart payload of `POST /api/page/{id}/capture`.
#[derive(Debug, Default)]
pub(crate) struct ArchiveUpload {
    pub title: Option<NonEmptyString>,
    pub html: Vec<u8>,
    pub screenshot: Option<Image>,
    pub metadata: Option<Value>,
    pub pending_id: Option<PendingId>,
    /// Fetch and store the document's sub-resources when true.
    pub save_assets: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Dedup {
    /// The served (rewritten) HTML blob already existed.
    pub html: bool,
    /// The exact uploaded HTML blob already existed.
    pub original_html: bool,
    pub screenshot: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArchiveResult {
    pub capture_id: i64,
    pub page_id: PageId,
    pub version: i64,
    pub deduplicated: Dedup,
    pub assets: AssetSummary,
}

fn meta_title(meta: Option<&Value>) -> Option<NonEmptyString> {
    let raw = meta?.get("title")?.as_str()?;
    NonEmptyString::trimmed(raw).ok()
}

#[expect(
    clippy::too_many_lines,
    reason = "the archive transaction is intentionally kept in one readable flow"
)]
pub(crate) async fn archive(
    state: &AppState,
    actor: &AuthUser,
    page_id: PageId,
    upload: ArchiveUpload,
) -> Result<ArchiveResult> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let started = std::time::Instant::now();
    let html_bytes = upload.html.len();
    if upload.html.is_empty() {
        return Err(AppError::invalid(
            "missing_html",
            "the html field is required and must not be empty",
        ));
    }

    let page = PageRecord::load(state.db.writer(), actor.id, page_id)
        .await?
        .or_not_found(Resource::Page)?;
    let page_url = url::Url::parse(&page.url).map_err(AppError::internal)?;

    let title = upload
        .title
        .or_else(|| meta_title(upload.metadata.as_ref()));
    let metadata_text = upload.metadata.as_ref().map(Value::to_string);

    // Network I/O happens before the write transaction.
    let (assets, asset_summary) = if upload.save_assets {
        collect_assets(
            state.db.writer(),
            &state.blobs,
            &state.fetch_client,
            &page_url,
            &upload.html,
        )
        .await?
    } else {
        (Vec::new(), AssetSummary::default())
    };

    let replacements: HashMap<String, String> = assets
        .iter()
        .map(|asset| (asset.url.clone(), asset.hash.content_path()))
        .collect();
    let rewritten = html::rewrite_resources(&upload.html, &page_url, &replacements);

    let original_blob =
        prepare_blob(state.db.writer(), &state.blobs, upload.html, "text/html").await?;
    let served_blob = match rewritten {
        Some(bytes) => {
            Some(prepare_blob(state.db.writer(), &state.blobs, bytes, "text/html").await?)
        }
        None => None,
    };
    let shot_blob = match upload.screenshot {
        Some(image) => {
            let format = image.format();
            let data = image.into_data();
            Some(prepare_blob(state.db.writer(), &state.blobs, data, format.mime_type()).await?)
        }
        None => None,
    };

    let mut tx = state.db.begin_write().await?;

    let original_id = insert_blob(&mut tx, &original_blob).await?;
    let html_id = match &served_blob {
        Some(blob) => insert_blob(&mut tx, blob).await?,
        None => original_id,
    };
    let shot_id = match &shot_blob {
        Some(blob) => Some(insert_blob(&mut tx, blob).await?),
        None => None,
    };

    let (capture_id, version) = NewCapture {
        owner: actor.id,
        page_id,
        content_id: html_id.into(),
        original_content_id: original_id.into(),
        screenshot_content_id: shot_id.map(Into::into),
        captured_at: now_ms(),
        title: title.as_deref(),
        content_type: "text/html",
        metadata: metadata_text.as_deref(),
    }
    .insert(&mut tx)
    .await?;

    insert_assets(&mut tx, capture_id, &assets).await?;

    if let Some(pending_id) = upload.pending_id
        && PendingRow::load(&mut *tx, actor.id, pending_id)
            .await?
            .is_some()
    {
        PendingRow::complete_into_page(&mut tx, actor.id, pending_id, page_id).await?;
    }

    tx.commit().await?;

    tracing::info!(
        capture_id = capture_id.get(),
        version,
        html_bytes,
        assets_saved = asset_summary.saved,
        assets_deduplicated = asset_summary.deduplicated,
        assets_failed = asset_summary.failed,
        assets_skipped = asset_summary.skipped,
        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "page captured"
    );

    Ok(ArchiveResult {
        capture_id: capture_id.get(),
        page_id,
        version,
        deduplicated: Dedup {
            html: served_blob
                .as_ref()
                .map_or(original_blob.existed, |blob| blob.existed),
            original_html: original_blob.existed,
            screenshot: shot_blob.as_ref().map(|blob| blob.existed),
        },
        assets: asset_summary,
    })
}
