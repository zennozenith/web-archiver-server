#![expect(
    dead_code,
    reason = "htmx request/response helpers cover the full protocol; not every variant is used yet"
)]

use std::convert::Infallible;

use askama::Template;
use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header, request::Parts},
    response::{Html, IntoResponse, Response},
};

use crate::error::AppError;

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
    ) -> HtmxResponse {
        if self.wants_partial() {
            HtmxResponse::partial(partial())
        } else {
            HtmxResponse::full(full())
        }
    }

    /// Choose full page or fragment. Only the chosen template is built and rendered.
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
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Ok(Self::from_headers(&parts.headers)))
    }
}

// ───────────────────────── Response side ─────────────────────────

/// A single `<hx-partial>` element: content swapped into its own target.
#[derive(Debug, Clone)]
pub(crate) struct PartialSwap {
    pub target: String,       // CSS selector, e.g. "#cart-count"
    pub swap: Option<String>, // e.g. "innerHTML", "outerHTML", "beforeend"
    pub html: String,
}

impl PartialSwap {
    pub(crate) fn new(target: impl Into<String>, html: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            swap: None,
            html: html.into(),
        }
    }
    pub(crate) fn swap(mut self, s: impl Into<String>) -> Self {
        self.swap = Some(s.into());
        self
    }

    fn render(&self) -> String {
        let swap = self
            .swap
            .as_deref()
            .map_or_else(String::new, |s| format!(r#" hx-swap="{}""#, escape_attr(s)));
        format!(
            r#"<hx-partial hx-target="{}"{}>{}</hx-partial>"#,
            escape_attr(&self.target),
            swap,
            self.html
        )
    }

    pub(crate) fn tpl<T: Template>(target: impl Into<String>, t: &T) -> Result<Self, AppError> {
        Ok(Self::new(target, t.render()?))
    }
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// Optional htmx response headers, attachable to any variant.
#[derive(Debug, Clone, Default)]
pub(crate) struct HxHeaders {
    pub retarget: Option<String>,
    pub reswap: Option<String>,
    pub reselect: Option<String>,
    pub trigger: Option<String>, // name or JSON
    pub trigger_after_swap: Option<String>,
    pub trigger_after_settle: Option<String>,
    pub push_url: Option<String>, // use "false" to prevent
    pub replace_url: Option<String>,
}

impl HxHeaders {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn retarget(mut self, v: impl Into<String>) -> Self {
        self.retarget = Some(v.into());
        self
    }
    pub(crate) fn reswap(mut self, v: impl Into<String>) -> Self {
        self.reswap = Some(v.into());
        self
    }
    pub(crate) fn reselect(mut self, v: impl Into<String>) -> Self {
        self.reselect = Some(v.into());
        self
    }
    pub(crate) fn trigger(mut self, v: impl Into<String>) -> Self {
        self.trigger = Some(v.into());
        self
    }
    pub(crate) fn push_url(mut self, v: impl Into<String>) -> Self {
        self.push_url = Some(v.into());
        self
    }
    pub(crate) fn replace_url(mut self, v: impl Into<String>) -> Self {
        self.replace_url = Some(v.into());
        self
    }

    fn apply(self, h: &mut HeaderMap) {
        let pairs = [
            ("hx-retarget", self.retarget),
            ("hx-reswap", self.reswap),
            ("hx-reselect", self.reselect),
            ("hx-trigger", self.trigger),
            ("hx-trigger-after-swap", self.trigger_after_swap),
            ("hx-trigger-after-settle", self.trigger_after_settle),
            ("hx-push-url", self.push_url),
            ("hx-replace-url", self.replace_url),
        ];
        for (name, val) in pairs {
            if let Some(v) = val {
                set(h, name, &v);
            }
        }
    }
}

fn set(h: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        h.insert(HeaderName::from_static(name), v);
    }
}

#[derive(Debug, Clone)]
pub(crate) enum HtmxResponse {
    /// Whole document (normal navigation, boosted nav, history restore, `hx-select`).
    Full(String),
    /// One fragment for the request's target.
    Partial(String),
    /// Several `<hx-partial>` elements, each swapped into its own target.
    Partials(Vec<PartialSwap>),
    /// 204: htmx ignores the body and does not swap.
    NoContent,
    /// Client-side redirect via `HX-Redirect` (full page load).
    Redirect(String),
    /// SPA-style redirect via `HX-Location` (no full reload).
    Location(String),
    /// `HX-Refresh: true`.
    Refresh,
    /// Non-2xx fragment, e.g. 422 validation errors. Pair with
    /// `hx-status:422="target:#errors"` on the client.
    Error { status: StatusCode, html: String },
}

impl HtmxResponse {
    pub(crate) fn full(html: impl Into<String>) -> Self {
        Self::Full(html.into())
    }
    pub(crate) fn partial(html: impl Into<String>) -> Self {
        Self::Partial(html.into())
    }
    pub(crate) fn partials(p: impl IntoIterator<Item = PartialSwap>) -> Self {
        Self::Partials(p.into_iter().collect())
    }
    pub(crate) fn error(status: StatusCode, html: impl Into<String>) -> Self {
        Self::Error {
            status,
            html: html.into(),
        }
    }

    // ───────────────────────── Askama helpers ─────────────────────────

    /// Attach extra htmx headers to this response.
    pub(crate) fn with(self, headers: HxHeaders) -> impl IntoResponse {
        let mut res = self.into_response();
        headers.apply(res.headers_mut());
        res
    }

    pub(crate) fn full_tpl<T: Template>(t: &T) -> Result<Self, AppError> {
        Ok(Self::Full(t.render()?))
    }

    pub(crate) fn partial_tpl<T: Template>(t: &T) -> Result<Self, AppError> {
        Ok(Self::Partial(t.render()?))
    }

    /// Non-2xx fragment, e.g. 422 validation errors.
    pub(crate) fn error_tpl<T: Template>(status: StatusCode, t: &T) -> Result<Self, AppError> {
        Ok(Self::Error {
            status,
            html: t.render()?,
        })
    }
}

impl IntoResponse for HtmxResponse {
    fn into_response(self) -> Response {
        let mut res = match self {
            Self::Full(html) | Self::Partial(html) => Html(html).into_response(),
            Self::Partials(items) => {
                let body: String = items.iter().map(PartialSwap::render).collect();
                Html(body).into_response()
            }
            Self::NoContent => StatusCode::NO_CONTENT.into_response(),
            Self::Redirect(url) => {
                let mut r = StatusCode::OK.into_response();
                set(r.headers_mut(), "hx-redirect", &url);
                r
            }
            Self::Location(url) => {
                let mut r = StatusCode::OK.into_response();
                set(r.headers_mut(), "hx-location", &url);
                r
            }
            Self::Refresh => {
                let mut r = StatusCode::OK.into_response();
                set(r.headers_mut(), "hx-refresh", "true");
                r
            }
            Self::Error { status, html } => (status, Html(html)).into_response(),
        };

        // The same URL can return a full page or a fragment, so caches must
        // key on these request headers.
        res.headers_mut().insert(
            header::VARY,
            HeaderValue::from_static("HX-Request, HX-Request-Type, HX-History-Restore-Request"),
        );
        res
    }
}
