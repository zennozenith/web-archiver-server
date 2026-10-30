//! HTTP fetching of page sub-resources.
//!
//! The original scratch implementation (see `examples/parallel_download.rs`)
//! wrote every response to disk and compressed it there. Archived content is
//! stored through `db::store`, which already compresses and deduplicates, so
//! this module only needs to fetch bytes and report the response metadata.
//!
//! Concurrency is handled by the caller with `futures::stream::buffer_unordered`.

use futures::StreamExt;
use reqwest::header::CONTENT_TYPE;
use url::Url;

/// Hard ceiling per resource, independent of the request body limit.
pub(crate) const MAX_ASSET_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum FetchError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("http status {0}")]
    Status(u16),
    #[error("resource exceeds {0} bytes")]
    TooLarge(usize),
    #[error("empty response body")]
    Empty,
}

#[derive(Debug)]
pub(crate) struct FetchedResource {
    /// URL after redirects (what the bytes actually came from).
    pub final_url: Url,
    pub status: u16,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Fetch one resource. Non-2xx responses are errors and are not stored.
pub(crate) async fn fetch(
    client: &reqwest::Client,
    url: Url,
    max_bytes: usize,
) -> Result<FetchedResource, FetchError> {
    let response = client.get(url.clone()).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::Status(status.as_u16()));
    }

    if response
        .content_length()
        .is_some_and(|len| len > u64::try_from(max_bytes).unwrap_or(u64::MAX))
    {
        return Err(FetchError::TooLarge(max_bytes));
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned();

    let final_url = response.url().clone();

    // Read incrementally so a server that lies about Content-Length (or streams
    // forever) cannot exhaust memory.
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(FetchError::TooLarge(max_bytes));
        }
        bytes.extend_from_slice(&chunk);
    }

    if bytes.is_empty() {
        return Err(FetchError::Empty);
    }

    Ok(FetchedResource {
        final_url,
        status: status.as_u16(),
        content_type,
        bytes,
    })
}
