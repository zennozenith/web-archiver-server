//! Serving stored blobs with Accept-Encoding negotiation and conditional requests.

use axum::{
    body::Body,
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{
            ACCEPT_ENCODING, ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_ENCODING,
            CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE, ETAG, IF_NONE_MATCH, VARY,
            X_CONTENT_TYPE_OPTIONS,
        },
    },
    response::{IntoResponse, Response},
};
use tokio_util::io::ReaderStream;

use crate::{
    blob_store::BlobStore,
    compress::Compression,
    db::contents::ContentRow,
    error::{AppError, Result, join_err},
};
use crate::{compress, helpers::to_hex};

/// Archived content is untrusted: no scripts, no framing tricks, no sniffing.
/// Loosen img-src/style-src if you want archived pages to load remote resources.
pub(crate) const CSP: &str =
    "sandbox; default-src 'none'; img-src data:; style-src 'unsafe-inline'";

/// Archived HTML is served from our own origin so it can load the rewritten
/// `/api/content/...` resources. Scripts stay disabled by `sandbox` (no
/// `allow-scripts`) and by an explicit `script-src 'none'`.
pub(crate) const CSP_ARCHIVED_HTML: &str = "sandbox; default-src 'none'; \
    script-src 'none'; connect-src 'none'; \
    img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
    font-src 'self' data:; media-src 'self'; frame-src 'self'";

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

pub(crate) fn etag_for(hash: &[u8]) -> String {
    format!("\"{}\"", to_hex(hash))
}

async fn decompress_blocking(data: Vec<u8>, comp: Compression) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || compress::decompress(&data, comp))
        .await
        .map_err(join_err)?
}

async fn compress_blocking(data: Vec<u8>, comp: Compression) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || compress::compress(&data, comp))
        .await
        .map_err(join_err)?
}

/// How the stored bytes become the response body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyPlan {
    /// The stored representation is exactly what the client accepts.
    Stored,
    /// Decode the stored representation; send the bytes as identity.
    Decode,
    /// Decode the stored representation and re-encode it with this algorithm.
    Transcode(Compression),
}

/// Pick the response representation and how to build it.
fn body_plan(comp: Compression, accept: &str) -> (Option<&'static str>, BodyPlan) {
    let stored_encoding = comp.content_encoding();
    let stored_accepted = stored_encoding.is_some_and(|e| accepts(accept, e));

    if comp == Compression::None {
        (None, BodyPlan::Stored)
    } else if stored_accepted {
        (stored_encoding, BodyPlan::Stored)
    } else if accepts(accept, "gzip") {
        // Browsers deliberately omit `br` over plain HTTP, while gzip is
        // accepted everywhere.
        (Some("gzip"), BodyPlan::Transcode(Compression::Gzip))
    } else {
        (None, BodyPlan::Decode)
    }
}

pub(crate) async fn serve_content(
    store: &BlobStore,
    req: &HeaderMap,
    row: ContentRow,
    content_type: &str,
    cache_control: &'static str,
) -> Result<Response> {
    serve_content_with_csp(store, req, row, content_type, cache_control, CSP).await
}

#[expect(
    clippy::too_many_arguments,
    reason = "the extra policy arguments keep the call sites explicit"
)]
#[expect(
    clippy::too_many_lines,
    reason = "response negotiation and body construction belong together"
)]
pub(crate) async fn serve_content_with_csp(
    store: &BlobStore,
    req: &HeaderMap,
    row: ContentRow,
    content_type: &str,
    cache_control: &'static str,
    csp: &'static str,
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

    // Prefer the representation we already have. If the client cannot take it,
    // transcode to gzip when possible instead of sending the bytes uncompressed.
    let (encoding, plan) = body_plan(comp, accept);

    // Strong ETag per representation (identity vs br vs gzip differ byte-wise).
    let etag = encoding.map_or_else(|| etag_for(&hash), |e| format!("\"{}-{e}\"", to_hex(&hash)));

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
    // Archived pages are often embedded or read from another origin (extension
    // UI, local tooling). Blobs are immutable and unauthenticated, so any origin
    // may read them; fonts in particular need this to load cross-origin.
    h.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    h.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp));
    if let Some(e) = encoding {
        h.insert(CONTENT_ENCODING, HeaderValue::from_static(e));
    }

    let body = match (data, storage_ref) {
        (Some(data), _) => Body::from(match plan {
            BodyPlan::Stored => data,
            BodyPlan::Decode => decompress_blocking(data, comp).await?,
            BodyPlan::Transcode(target) => {
                let decoded = decompress_blocking(data, comp).await?;
                compress_blocking(decoded, target).await?
            }
        }),
        // On disk and sendable as-is: stream it, never hold the whole file in memory.
        (None, Some(r)) if matches!(plan, BodyPlan::Stored) => {
            let file = store.open_file(&r).await?;
            h.insert(
                CONTENT_LENGTH,
                HeaderValue::from(u64::try_from(size_stored).unwrap_or_default()),
            );
            Body::from_stream(ReaderStream::new(file))
        }
        // On disk but the client can't take the stored encoding: read, decode,
        // and (if requested) re-encode.
        (None, Some(r)) => {
            let raw = store.read(&r).await?;
            let decoded = decompress_blocking(raw, comp).await?;
            Body::from(match plan {
                BodyPlan::Decode | BodyPlan::Stored => decoded,
                BodyPlan::Transcode(target) => compress_blocking(decoded, target).await?,
            })
        }
        (None, None) => {
            return Err(AppError::Internal(
                "content row has neither data nor storage_ref".into(),
            ));
        }
    };

    Ok((StatusCode::OK, h, body).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_stored_encoding_passes_through() {
        let (encoding, plan) = body_plan(Compression::Brotli, "gzip, deflate, br");
        assert_eq!(encoding, Some("br"));
        assert_eq!(plan, BodyPlan::Stored);

        let (encoding_gzip, plan_gzip) = body_plan(Compression::Gzip, "gzip, deflate");
        assert_eq!(encoding_gzip, Some("gzip"));
        assert_eq!(plan_gzip, BodyPlan::Stored);
    }

    #[test]
    fn brotli_is_transcoded_to_gzip_for_http_clients() {
        let (encoding, plan) = body_plan(Compression::Brotli, "gzip, deflate");
        assert_eq!(encoding, Some("gzip"));
        assert_eq!(plan, BodyPlan::Transcode(Compression::Gzip));
    }

    #[test]
    fn missing_encodings_fall_back_to_identity() {
        let (encoding, plan) = body_plan(Compression::Brotli, "");
        assert_eq!(encoding, None);
        assert_eq!(plan, BodyPlan::Decode);
    }

    #[test]
    fn uncompressed_content_is_always_identity() {
        let (encoding, plan) = body_plan(Compression::None, "gzip, br");
        assert_eq!(encoding, None);
        assert_eq!(plan, BodyPlan::Stored);
    }

    #[test]
    fn q_zero_disables_an_encoding() {
        let (encoding, plan) = body_plan(Compression::Brotli, "br;q=0, gzip");
        assert_eq!(encoding, Some("gzip"));
        assert_eq!(plan, BodyPlan::Transcode(Compression::Gzip));
    }
}
