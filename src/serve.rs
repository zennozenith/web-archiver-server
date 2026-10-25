//! Serving stored blobs with Accept-Encoding negotiation and conditional requests.

use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, header::*},
    response::{IntoResponse, Response},
};

use crate::compress;
use crate::error::{AppError, Result, join_err};
use crate::models::{Compression, ContentRow};

/// Archived content is untrusted: no scripts, no framing tricks, no sniffing.
/// Loosen img-src/style-src if you want archived pages to load remote resources.
const CSP: &str = "sandbox; default-src 'none'; img-src data:; style-src 'unsafe-inline'";

pub const COLS: &str = "ct.id, ct.hash, ct.mime_type, ct.size_original, ct.size_stored, \
                        ct.compression_id, ct.data, ct.storage_ref, ct.created_at";

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

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
                .map_or(false, |v| v == 0.0)
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

pub async fn serve_content(
    req: &HeaderMap,
    row: ContentRow,
    content_type: &str,
    cache_control: &'static str,
) -> Result<Response> {
    let accept = req
        .get(ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let comp = row.compression_id;

    // Send stored bytes untouched if the client understands them.
    let encoding: Option<&'static str> = comp.content_encoding().filter(|e| accepts(accept, e));

    // Strong ETag per representation (identity vs br vs gzip differ byte-wise).
    let etag = match encoding {
        Some(e) => format!("\"{}-{e}\"", to_hex(&row.hash)),
        None => etag_for(&row.hash),
    };

    let mut h = HeaderMap::new();
    h.insert(ETAG, hv(&etag)?);
    h.insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    h.insert(VARY, HeaderValue::from_static("Accept-Encoding"));

    if let Some(inm) = req.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if etag_matches(inm, &etag) {
            return Ok((StatusCode::NOT_MODIFIED, h).into_response());
        }
    }

    let data = row
        .data
        .ok_or_else(|| AppError::Internal("storage_ref content is not implemented yet".into()))?;

    let body = if comp == Compression::None || encoding.is_some() {
        data
    } else {
        tokio::task::spawn_blocking(move || compress::decompress(&data, comp))
            .await
            .map_err(join_err)??
    };

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

    Ok((StatusCode::OK, h, body).into_response())
}
