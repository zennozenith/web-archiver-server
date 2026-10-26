use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[repr(i64)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    None = 0,
    Gzip = 1,
    Brotli = 2,
    // Zstd = 3,  <- add here + INSERT into the compression table
}

impl Compression {
    pub fn content_encoding(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Gzip => Some("gzip"),
            Self::Brotli => Some("br"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, strum::Display)]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PendingStatus {
    Queued,
    InProgress,
    Failed,
}

#[derive(Debug, FromRow)]
pub struct ContentRow {
    pub id: i64,
    pub hash: Vec<u8>, // 32 bytes
    pub mime_type: Option<String>,
    pub size_original: i64,
    pub size_stored: i64,
    pub compression_id: Compression,
    pub data: Option<Vec<u8>>,
    pub storage_ref: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, FromRow)]
pub struct UrlRow {
    pub id: i64,
    pub url: String,
    pub scheme: String,
    pub host: String,
    pub path: String,
    pub query: Option<String>,
}

#[derive(Debug, FromRow)]
pub struct CaptureRow {
    pub id: i64,
    pub url_id: i64,
    pub content_id: Option<i64>,
    pub screenshot_content_id: Option<i64>,
    pub captured_at: i64,
    pub status_code: Option<i64>,
    pub redirect_url_id: Option<i64>,
    pub content_type: Option<String>,
    pub original_encoding: Option<String>,
    pub request_headers: Option<String>,  // JSON text
    pub response_headers: Option<String>, // JSON text
    pub metadata: Option<String>,         // JSON text
    pub title: Option<String>,
    pub crawl_id: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagRow {
    pub id: i64,
    pub name: String,
    pub color: Option<String>, // palette key, never a hex value
    pub created_at: i64,
}

#[derive(Debug, FromRow)]
pub struct PendingRow {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub status: PendingStatus,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
}
