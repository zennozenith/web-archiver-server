//! Parsing of htmx request headers.

use std::convert::Infallible;

use askama::Template;
use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, request::Parts},
};

use super::{
    HtmxResponse,
    header::{self, name},
};
use crate::prelude::*;

/// Value of the `HX-Request-Type` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestType {
    /// The target is `body`, or `hx-select` is used: send a whole page.
    Full,
    /// The target is any other element: send a fragment.
    Partial,
}

/// Parsed htmx 4 request headers.
///
/// Extraction never fails, so this is safe on routes that serve both plain
/// browser requests and htmx requests.
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors the boolean htmx request headers"
)]
#[derive(Debug, Clone, Default)]
pub(crate) struct HtmxRequest {
    /// `HX-Request`: the request was issued by htmx.
    pub is_htmx: bool,
    /// `HX-Boosted`: the request comes from a boosted link or form.
    pub boosted: bool,
    /// `HX-History-Restore-Request`: htmx is restoring history after a miss.
    pub history_restore: bool,
    /// `HX-Request-Type`, when sent.
    pub request_type: Option<RequestType>,
    /// `HX-Target`: identifier of the swap target.
    pub target: Option<String>,
    /// `HX-Source`: identifier of the element that triggered the request.
    pub source: Option<String>,
    /// `HX-Current-URL`: the browser's current URL.
    pub current_url: Option<String>,
}

impl HtmxRequest {
    /// Builds a request description from raw headers.
    pub(crate) fn from_headers(h: &HeaderMap) -> Self {
        let owned = |n| header::get(h, n).map(str::to_owned);
        Self {
            is_htmx: header::is_true(h, name::REQUEST),
            boosted: header::is_true(h, name::BOOSTED),
            history_restore: header::is_true(h, name::HISTORY_RESTORE_REQUEST),
            request_type: match header::get(h, name::REQUEST_TYPE) {
                Some("full") => Some(RequestType::Full),
                Some("partial") => Some(RequestType::Partial),
                _ => None,
            },
            target: owned(name::TARGET),
            source: owned(name::SOURCE),
            current_url: owned(name::CURRENT_URL),
        }
    }

    /// Whether the response should be a fragment.
    ///
    /// A history restore always needs the full page, even though
    /// `HX-Request` is set.
    pub(crate) fn wants_partial(&self) -> bool {
        self.is_htmx && !self.history_restore && self.request_type == Some(RequestType::Partial)
    }

    /// Picks the full-page or fragment variant from pre-rendered strings.
    /// The closures are lazy, so only the needed one runs.
    pub(crate) fn render(
        &self,
        full: impl FnOnce() -> String,
        partial: impl FnOnce() -> String,
    ) -> HtmxResponse {
        if self.wants_partial() {
            HtmxResponse::partial(partial())
        } else {
            HtmxResponse::full(full())
        }
    }

    /// Like [`render`](Self::render), but takes askama templates. Only the
    /// chosen template is built and rendered.
    ///
    /// # Errors
    ///
    /// Returns an error if the chosen template fails to render.
    pub(crate) fn render_tpl<F: Template, P: Template>(
        &self,
        full: impl FnOnce() -> F,
        partial: impl FnOnce() -> P,
    ) -> Result<HtmxResponse, AppError> {
        if self.wants_partial() {
            HtmxResponse::partial_tpl(&partial())
        } else {
            HtmxResponse::full_tpl(&full())
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
