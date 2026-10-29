use crate::{extract::comma_separated_or_seq, models::*};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

// ── Requests ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreatePending {
    pub url: String,
    pub title: Option<String>,
    #[serde(default, deserialize_with = "comma_separated_or_seq")]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PatchPending {
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<PendingStatus>,
}

#[derive(Debug, Deserialize)]
pub struct FailPending {
    pub error: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTag {
    pub name: String,
    pub color: Option<String>,
}

/// PATCH semantics: `color: null` clears, absent leaves unchanged.
#[derive(Debug, Deserialize)]
pub struct PatchTag {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub color: Option<Option<String>>,
}

fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(d).map(Some)
}

#[derive(Debug, Deserialize)]
pub struct ReplaceTags {
    pub tags: Vec<CreateTag>,
}

#[derive(Debug, Deserialize)]
pub struct MergeTag {
    pub into: i64,
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub tag: Vec<String>, // needs axum-extra's Query for repeated keys
    #[serde(default, rename = "match")]
    pub match_mode: MatchMode,
    pub host: Option<String>,
    pub q: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    #[default]
    All,
    Any,
}

/// Parsed from the multipart fields of POST /api/archive
#[derive(Debug, Default)]
pub struct ArchiveUpload {
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
pub enum ScreenshotFormat {
    Png,
    Jpeg,
}

impl ScreenshotFormat {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
    /// Verify claimed format against magic bytes.
    pub fn matches(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(&[0x89, b'P', b'N', b'G']),
            Self::Jpeg => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        }
    }
}

// ── Responses ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveResponse {
    pub capture_id: i64,
    pub page_id: i64,
    pub version: i64,
    pub deduplicated: Dedup,
    pub assets: AssetSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dedup {
    pub html: bool,
    pub screenshot: Option<bool>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetSummary {
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
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDto {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub status: PendingStatus,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub tags: Vec<TagRow>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSummary {
    pub id: i64,
    pub version: i64,
    pub captured_at: i64,
    pub title: Option<String>,
    pub status_code: Option<i64>,
    pub content_hash: Option<String>, // hex
    pub has_screenshot: bool,
}
