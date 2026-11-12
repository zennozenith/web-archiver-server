#![expect(
    dead_code,
    reason = "htmx request/response helpers cover the full protocol; not every variant is used yet"
)]

use std::convert::Infallible;

use askama::Template;
use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, request::Parts},
};

use crate::prelude::*;

// ───────────────────────── Request side ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestType {
    /// Target is `body`, or `hx-select` is used: send a whole page.
    Full,
    /// Target is any other element: send a fragment.
    Partial,
}

/// Parsed htmx 4 request headers. Always extracts successfully, so it is
/// safe to use on routes that serve both normal browser and htmx requests.
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors the boolean htmx request headers"
)]
#[derive(Debug, Clone, Default)]
pub(crate) struct HtmxRequest {
    pub is_htmx: bool,
    pub boosted: bool,
    pub history_restore: bool,
    pub request_type: Option<RequestType>,
    pub target: Option<String>,
    pub source: Option<String>,
    pub current_url: Option<String>,
}

fn hdr<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

fn hdr_true(h: &HeaderMap, name: &str) -> bool {
    hdr(h, name) == Some("true")
}

impl HtmxRequest {
    pub(crate) fn from_headers(h: &HeaderMap) -> Self {
        Self {
            is_htmx: hdr_true(h, "hx-request"),
            boosted: hdr_true(h, "hx-boosted"),
            history_restore: hdr_true(h, "hx-history-restore-request"),
            request_type: match hdr(h, "hx-request-type") {
                Some("full") => Some(RequestType::Full),
                Some("partial") => Some(RequestType::Partial),
                _ => None,
            },
            target: hdr(h, "hx-target").map(str::to_owned),
            source: hdr(h, "hx-source").map(str::to_owned),
            current_url: hdr(h, "hx-current-url").map(str::to_owned),
        }
    }

    /// True only when a fragment should be returned. A history restore needs
    /// the full page even though `HX-Request` is set.
    pub(crate) fn wants_partial(&self) -> bool {
        self.is_htmx && !self.history_restore && self.request_type == Some(RequestType::Partial)
    }

    /// Pick the right response variant. Closures are lazy, so only the
    /// needed template is rendered.
    pub(crate) fn render(
        &self,
        full: impl FnOnce() -> String,
        partial: impl FnOnce() -> String,
    ) -> super::HtmxResponse {
        if self.wants_partial() {
            super::HtmxResponse::partial(partial())
        } else {
            super::HtmxResponse::full(full())
        }
    }

    /// Choose full page or fragment. Only the chosen template is built and rendered.
    pub(crate) fn render_tpl<F: Template, P: Template>(
        &self,
        full: impl FnOnce() -> F,
        partial: impl FnOnce() -> P,
    ) -> Result<super::HtmxResponse, AppError> {
        if self.wants_partial() {
            super::HtmxResponse::partial_tpl(&partial())
        } else {
            super::HtmxResponse::full_tpl(&full())
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for HtmxRequest {
    type Rejection = Infallible;
    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl Future<Output = Result<Self, Infallible>> {
        std::future::ready(Ok(Self::from_headers(&parts.headers)))
    }
}
