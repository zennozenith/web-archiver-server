//! JSON shapes for archive captures and their blobs.

use serde::Serialize;
use serde_json::Value;

use crate::{
    db::{
        capture::{CaptureDetail, CaptureId},
        page::PageId,
    },
    infra::Blake3Hash,
    util::mime::MimeType,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlobSummary {
    pub(crate) hash: Blake3Hash,
    pub(crate) size_original: i64,
    pub(crate) size_stored: i64,
    pub(crate) mime_type: MimeType,
}

impl BlobSummary {
    /// `None` when the capture has no such blob (no hash).
    pub(crate) fn new(
        hash: Option<Blake3Hash>,
        size_original: Option<i64>,
        size_stored: Option<i64>,
        mime_type: MimeType,
    ) -> Option<Self> {
        if let (Some(hash), Some(size_original), Some(size_stored)) =
            (hash, size_original, size_stored)
        {
            Some(Self {
                hash,
                size_original,
                size_stored,
                mime_type,
            })
        } else {
            None
        }
    }

    pub(crate) fn html(
        hash: Option<Blake3Hash>,
        size: Option<i64>,
        stored: Option<i64>,
    ) -> Option<Self> {
        Self::new(
            hash,
            size,
            stored,
            "text/html".parse::<MimeType>().unwrap_or_default(),
        )
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureDetailResponse {
    pub(crate) id: CaptureId,
    pub(crate) page_id: PageId,
    pub(crate) url: String,
    pub(crate) version: i64,
    pub(crate) captured_at: i64,
    pub(crate) title: Option<String>,
    pub(crate) status_code: Option<i64>,
    pub(crate) content_type: Option<String>,
    pub(crate) original_encoding: Option<String>,
    pub(crate) redirect_url: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) request_headers: Option<Value>,
    pub(crate) response_headers: Option<Value>,
    pub(crate) metadata: Option<Value>,
    pub(crate) html: Option<BlobSummary>,
    pub(crate) original_html: Option<BlobSummary>,
    pub(crate) screenshot: Option<BlobSummary>,
    pub(crate) asset_count: i64,
}

/// Parses a JSON text column; malformed values are reported as absent.
fn parse_json_column(raw: Option<String>) -> Option<Value> {
    raw.and_then(|text| serde_json::from_str(&text).ok())
}

impl From<CaptureDetail> for CaptureDetailResponse {
    fn from(d: CaptureDetail) -> Self {
        Self {
            id: d.id,
            page_id: d.page_id,
            url: d.url,
            version: d.version,
            captured_at: d.captured_at,
            title: d.title,
            status_code: d.status_code,
            content_type: d.content_type,
            original_encoding: d.original_encoding,
            redirect_url: d.redirect_url,
            error: d.error,
            request_headers: parse_json_column(d.request_headers),
            response_headers: parse_json_column(d.response_headers),
            metadata: parse_json_column(d.metadata),
            html: BlobSummary::html(d.html_hash, d.html_size, d.html_stored),
            original_html: BlobSummary::html(
                d.original_html_hash,
                d.original_html_size,
                d.original_html_stored,
            ),
            screenshot: BlobSummary::new(
                d.screenshot_hash,
                d.screenshot_size,
                d.screenshot_stored,
                d.screenshot_mime,
            ),
            asset_count: d.asset_count,
        }
    }
}
