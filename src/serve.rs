//! Serving stored blobs with Accept-Encoding negotiation and conditional requests.

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header::*},
    response::{IntoResponse, Response},
};
use tokio_util::io::ReaderStream;

use crate::models::{Compression, ContentRow};
use crate::{
    blob_store::BlobStore,
    error::{AppError, Result, join_err},
};
use crate::{compress, db::helpers::to_hex};

/// Archived content is untrusted: no scripts, no framing tricks, no sniffing.
/// Loosen img-src/style-src if you want archived pages to load remote resources.
const CSP: &str = "sandbox; default-src 'none'; img-src data:; style-src 'unsafe-inline'";

pub const COLS: &str = "ct.id, ct.hash, ct.mime_type, ct.size_original, ct.size_stored, \
                        ct.compression_id, ct.data, ct.storage_ref, ct.created_at";

fn accepts(header: &str, coding: &str) -> bool {
    header.split(',').any(|part| {
        let mut it = part.split(';');
        if !it.next().unwrap_or("").trim().eq_ignore_ascii_case(coding) {
            return false;
        }
        // "q=0" means explicitly not acceptable
        !it.any(|p| {
            p.trim()
                .strip_prefix("q=")
                .and_then(|q| q.trim().parse::<f32>().ok())
                == Some(0.0)
        })
    })
}

fn etag_matches(if_none_match: &str, etag: &str) -> bool {
    if_none_match
        .split(',')
        .map(str::trim)
        .any(|t| t == "*" || t.trim_start_matches("W/") == etag)
}

fn hv(s: &str) -> Result<HeaderValue> {
    HeaderValue::from_str(s).map_err(|e| AppError::Internal(Box::new(e)))
}

pub fn etag_for(hash: &[u8]) -> String {
    format!("\"{}\"", to_hex(hash))
}

async fn decompress_blocking(data: Vec<u8>, comp: Compression) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || compress::decompress(&data, comp))
        .await
        .map_err(join_err)?
}

pub async fn serve_content(
    store: &BlobStore,
    req: &HeaderMap,
    row: ContentRow,
    content_type: &str,
    cache_control: &'static str,
) -> Result<Response> {
    let ContentRow {
        hash,
        size_stored,
        compression_id: comp,
        data,
        storage_ref,
        ..
    } = row;

    let accept = req
        .get(ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Send stored bytes untouched if the client understands them.
    let encoding: Option<&'static str> = comp.content_encoding().filter(|e| accepts(accept, e));
    let passthrough = comp == Compression::None || encoding.is_some();

    // Strong ETag per representation (identity vs br vs gzip differ byte-wise).
    let etag = match encoding {
        Some(e) => format!("\"{}-{e}\"", to_hex(&hash)),
        None => etag_for(&hash),
    };

    let mut h = HeaderMap::new();
    h.insert(ETAG, hv(&etag)?);
    h.insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    h.insert(VARY, HeaderValue::from_static("Accept-Encoding"));

    if let Some(inm) = req.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok())
        && etag_matches(inm, &etag)
    {
        return Ok((StatusCode::NOT_MODIFIED, h).into_response());
    }

    h.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    if let Some(e) = encoding {
        h.insert(CONTENT_ENCODING, HeaderValue::from_static(e));
    }

    let body = match (data, storage_ref) {
        (Some(data), _) => Body::from(if passthrough {
            data
        } else {
            decompress_blocking(data, comp).await?
        }),
        // On disk and sendable as-is: stream it, never hold the whole file in memory.
        (None, Some(r)) if passthrough => {
            let file = store.open_file(&r).await?;
            h.insert(CONTENT_LENGTH, HeaderValue::from(size_stored as u64));
            Body::from_stream(ReaderStream::new(file))
        }
        // On disk but the client can't take the stored encoding: read + decompress.
        (None, Some(r)) => {
            let raw = store.read(&r).await?;
            Body::from(decompress_blocking(raw, comp).await?)
        }
        (None, None) => {
            return Err(AppError::Internal(
                "content row has neither data nor storage_ref".into(),
            ));
        }
    };

    Ok((StatusCode::OK, h, body).into_response())
}
