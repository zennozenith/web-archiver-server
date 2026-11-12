#![expect(
    dead_code,
    reason = "htmx request/response helpers cover the full protocol; not every variant is used yet"
)]

use askama::Template;
use axum::{
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};

use crate::error::AppError;

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
    pub trigger: Option<String>,              // name or JSON
    pub trigger_after_swap: Option<String>,   // TODO:
    pub trigger_after_settle: Option<String>, // TODO:
    pub push_url: Option<String>,             // use "false" to prevent
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

#[derive(Debug, Clone, Copy)]
pub(crate) enum ToastKind {
    Success,
    Error,
}

impl ToastKind {
    const fn class(self) -> &'static str {
        match self {
            Self::Success => "alert-success",
            Self::Error => "alert-error",
        }
    }
}

#[derive(Debug, Template)]
#[template(path = "partials/toast.templ.html")]
struct ToastTpl {
    kind: &'static str,
    message: String,
}

/// Builds one response out of several rendered templates, e.g. the main
/// fragment + out-of-band updates + a toast. Order matters: main content first.
#[derive(Debug, Default)]
pub(crate) struct Fragments {
    body: String,
    headers: HeaderMap,
    status: Option<StatusCode>,
}

impl Fragments {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push<T: Template>(mut self, tpl: &T) -> Result<Self, AppError> {
        self.body.push_str(&tpl.render()?);
        Ok(self)
    }

    /// Appends a toast (out-of-band, lands in `#toasts`).
    pub(crate) fn toast(self, kind: ToastKind, message: &str) -> Result<Self, AppError> {
        self.push(&ToastTpl {
            kind: kind.class(),
            message: message.to_owned(),
        })
    }

    /// Adds a response header. `name` must be lowercase.
    pub(crate) fn header(mut self, name: &'static str, value: &'static str) -> Self {
        self.headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
        self
    }

    pub(crate) const fn status(mut self, status: StatusCode) -> Self {
        self.status = Some(status);
        self
    }
}

impl IntoResponse for Fragments {
    fn into_response(self) -> Response {
        let status = self.status.unwrap_or(StatusCode::OK);
        (status, self.headers, Html(self.body)).into_response()
    }
}
