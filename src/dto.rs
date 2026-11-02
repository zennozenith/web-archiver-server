use crate::{
    db::{pending::PendingStatus, tags::TagRow},
    extract::comma_separated_or_seq,
    limit::Limit,
};
use serde::{Deserialize, Serialize};

// ── Requests ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct CreatePending {
    pub url: String,
    pub title: Option<String>,
    #[serde(default, deserialize_with = "comma_separated_or_seq")]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PatchPending {
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<PendingStatus>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FailPending {
    pub error: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PageQuery {
    #[serde(default)]
    pub tag: Vec<String>, // needs axum-extra's Query for repeated keys
    #[serde(default, rename = "match")]
    pub match_mode: MatchMode,
    pub host: Option<String>,
    pub q: Option<String>,
    pub limit: Option<Limit>,
    pub cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MatchMode {
    #[default]
    All,
    Any,
}

/// Parsed from the multipart fields of POST /api/archive
#[derive(Debug, Default)]
pub(crate) struct ArchiveUpload {
    pub html: Vec<u8>,
    pub screenshot: Option<(Vec<u8>, ScreenshotFormat)>,
    pub metadata: Option<serde_json::Value>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub pending_id: Option<i64>,
    /// `save-assets=false` skips fetching sub-resources. Defaults to true.
    pub save_assets: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ScreenshotFormat {
    Png,
    Jpeg,
}

impl ScreenshotFormat {
    pub(crate) const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
    /// Verify claimed format against magic bytes.
    pub(crate) fn matches(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(&[0x89, b'P', b'N', b'G']),
            Self::Jpeg => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        }
    }
}

// ── Responses ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArchiveResponse {
    pub capture_id: i64,
    pub page_id: i64,
    pub version: i64,
    pub deduplicated: Dedup,
    pub assets: AssetSummary,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingDto {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub status: PendingStatus,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub tags: Vec<TagRow>,
}

#[derive(Debug)]
pub(crate) struct Cursor {
    pub ts: i64,
    pub id: i64,
}
