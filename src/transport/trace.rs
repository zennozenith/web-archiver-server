//! Per-request tracing: the `http_request` span, the access-log line and
//! request-id handling.
//!
//! Every event logged while a request is handled (handlers, services, SQL,
//! errors) is emitted inside the `http_request` span, so in JSON output each
//! line carries `request_id`, `route`, `client_ip` and, once known, `user_id`.
//! One `grep request_id=...` reconstructs a whole request.
//!
//! Privacy rules, deliberately strict:
//! * the span records the **path only**, never the query string (e-mailed
//!   links carry `?token=...`);
//! * headers (`Authorization`, `Cookie`) and bodies are never logged;
//! * a client-supplied `X-Request-Id` is accepted for correlation but capped
//!   in length, so it cannot bloat the logs.

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::MatchedPath,
    http::{Request, Response},
};
use tower_http::{classify::ServerErrorsFailureClass, trace::TraceLayer};
use tracing::{Span, field};

use crate::{app::RuntimeConfig, identity::client_ip};

/// Longest client-supplied request id that is echoed into logs.
const MAX_REQUEST_ID_LEN: usize = 128;

/// Wrap every route of `router` in the request span and access log.
///
/// Must sit *inside* `SetRequestIdLayer`, which generates the id this span
/// reads. (`Router::layer` makes later layers outer, so add it first.)
pub(crate) fn trace_requests<S>(router: Router<S>, config: &Arc<RuntimeConfig>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let config = Arc::clone(config);
    let make_span = move |request: &Request<Body>| request_span(request, &config);

    router.layer(
        TraceLayer::new_for_http()
            .make_span_with(make_span)
            // The access line is written once, on response.
            .on_request(())
            .on_response(on_response)
            .on_failure(on_failure),
    )
}

fn request_span(request: &Request<Body>, config: &RuntimeConfig) -> Span {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|id| id.len() <= MAX_REQUEST_ID_LEN)
        .unwrap_or("unknown");
    // The route template (`/api/capture/{id}`) has bounded cardinality, which
    // makes it the right key for grouping and metrics; `path` is the concrete
    // URL for debugging a single request.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str);
    let ip = client_ip(request.extensions(), request.headers(), config);

    tracing::info_span!(
        "http_request",
        request_id,
        method = %request.method(),
        route,
        path = %request.uri().path(),
        client_ip = ip.map(field::display),
        // Filled in later, once known: by the `AuthUser` extractor and by
        // `AppError::into_response`.
        user_id = field::Empty,
        error_code = field::Empty,
    )
}

/// The access-log line. Level follows the outcome so `RUST_LOG`/alerting can
/// key off it: 5xx is `error`, 4xx is `warn`, everything else `info`.
fn on_response(response: &Response<Body>, latency: Duration, span: &Span) {
    let status = response.status();
    let latency_ms = u64::try_from(latency.as_millis()).unwrap_or(u64::MAX);

    if status.is_server_error() {
        tracing::error!(parent: span, status = status.as_u16(), latency_ms, "request finished");
    } else if status.is_client_error() {
        tracing::warn!(parent: span, status = status.as_u16(), latency_ms, "request finished");
    } else {
        tracing::info!(parent: span, status = status.as_u16(), latency_ms, "request finished");
    }
}

/// Failures that never produced a response (dropped connection, body stream
/// error). 5xx *responses* are classified as failures too, but those are
/// already covered by [`on_response`], so they are ignored here.
fn on_failure(failure: ServerErrorsFailureClass, latency: Duration, span: &Span) {
    match failure {
        ServerErrorsFailureClass::StatusCode(_) => {}
        ServerErrorsFailureClass::Error(error) => {
            let latency_ms = u64::try_from(latency.as_millis()).unwrap_or(u64::MAX);
            tracing::error!(parent: span, %error, latency_ms, "request aborted");
        }
    }
}
