//! Single-response variants and optional htmx response headers.

use askama::Template;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};

use super::header::{self, name};
use crate::error::AppError;

/// Escapes a value for use inside a double-quoted HTML attribute.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// One `<hx-partial>` element: content swapped into its own target.
#[derive(Debug, Clone)]
pub(crate) struct PartialSwap {
    /// CSS selector of the target, e.g. `#cart-count`.
    pub target: String,
    /// Optional swap style, e.g. `innerHTML`, `outerHTML`, `beforeend`.
    pub swap: Option<String>,
    /// Markup to swap in.
    pub html: String,
}

impl PartialSwap {
    /// Creates a partial using the target's default swap style.
    pub(crate) fn new(target: impl Into<String>, html: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            swap: None,
            html: html.into(),
        }
    }

    /// Creates a partial from an askama template.
    ///
    /// # Errors
    ///
    /// Returns an error if the template fails to render.
    pub(crate) fn tpl<T: Template>(target: impl Into<String>, t: &T) -> Result<Self, AppError> {
        Ok(Self::new(target, t.render()?))
    }

    /// Overrides the swap style.
    pub(crate) fn swap(mut self, style: impl Into<String>) -> Self {
        self.swap = Some(style.into());
        self
    }

    fn render(&self) -> String {
        let swap = self
            .swap
            .as_deref()
            .map_or_else(String::new, |s| format!(r#" hx-swap="{}""#, escape_attr(s)));
        format!(
            r#"<hx-partial hx-target="{}"{swap}>{}</hx-partial>"#,
            escape_attr(&self.target),
            self.html,
        )
    }
}

/// Optional htmx response headers, attachable to [`HtmxResponse`] or
/// [`Fragments`](super::Fragments).
#[derive(Debug, Clone, Default)]
pub(crate) struct HxHeaders {
    retarget: Option<String>,
    reswap: Option<String>,
    reselect: Option<String>,
    trigger: Option<String>,
    trigger_after_swap: Option<String>,
    trigger_after_settle: Option<String>,
    push_url: Option<String>,
    replace_url: Option<String>,
}

macro_rules! hx_setters {
    ($($(#[$doc:meta])* $field:ident),* $(,)?) => {
        $(
            $(#[$doc])*
            pub(crate) fn $field(mut self, v: impl Into<String>) -> Self {
                self.$field = Some(v.into());
                self
            }
        )*
    };
}

impl HxHeaders {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    hx_setters! {
        /// `HX-Retarget`: CSS selector overriding the swap target.
        retarget,
        /// `HX-Reswap`: overrides the swap style (`none` skips the swap).
        reswap,
        /// `HX-Reselect`: CSS selector choosing which part of the response to swap.
        reselect,
        /// `HX-Trigger`: event name or JSON object, fired on receipt.
        trigger,
        /// `HX-Trigger-After-Swap`: event fired after the swap.
        trigger_after_swap,
        /// `HX-Trigger-After-Settle`: event fired after the settle step.
        trigger_after_settle,
        /// `HX-Push-Url`: URL to push to history, or `false` to prevent.
        push_url,
        /// `HX-Replace-Url`: URL to replace in history, or `false` to prevent.
        replace_url,
    }

    pub(super) fn apply_to(self, h: &mut HeaderMap) {
        let pairs = [
            (name::RETARGET, self.retarget),
            (name::RESWAP, self.reswap),
            (name::RESELECT, self.reselect),
            (name::TRIGGER, self.trigger),
            (name::TRIGGER_AFTER_SWAP, self.trigger_after_swap),
            (name::TRIGGER_AFTER_SETTLE, self.trigger_after_settle),
            (name::PUSH_URL, self.push_url),
            (name::REPLACE_URL, self.replace_url),
        ];
        for (name, value) in pairs {
            if let Some(v) = value {
                header::set(h, name, &v);
            }
        }
    }
}

/// Adds `HX-Redirect` (full page load) to an existing response.
pub(crate) fn with_redirect(mut resp: Response, to: &str) -> Response {
    header::set(resp.headers_mut(), name::REDIRECT, to);
    resp
}

/// Every shape of response an htmx-aware route can return.
#[derive(Debug, Clone)]
pub(crate) enum HtmxResponse {
    /// Whole document (navigation, boosted nav, history restore, `hx-select`).
    Full(String),
    /// One fragment for the request's target.
    Partial(String),
    /// Several `<hx-partial>` elements, each swapped into its own target.
    Partials(Vec<PartialSwap>),
    /// 204: htmx ignores the body and does not swap.
    NoContent,
    /// `HX-Redirect`: client-side redirect with a full page load.
    Redirect(String),
    /// `HX-Location`: SPA-style redirect without a full reload.
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

    /// Renders a full page from a template.
    ///
    /// # Errors
    ///
    /// Returns an error if the template fails to render.
    pub(crate) fn full_tpl<T: Template>(t: &T) -> Result<Self, AppError> {
        Ok(Self::Full(t.render()?))
    }

    /// Renders a fragment from a template.
    ///
    /// # Errors
    ///
    /// Returns an error if the template fails to render.
    pub(crate) fn partial_tpl<T: Template>(t: &T) -> Result<Self, AppError> {
        Ok(Self::Partial(t.render()?))
    }

    /// Renders a non-2xx fragment (e.g. validation errors) from a template.
    ///
    /// # Errors
    ///
    /// Returns an error if the template fails to render.
    pub(crate) fn error_tpl<T: Template>(status: StatusCode, t: &T) -> Result<Self, AppError> {
        Ok(Self::Error {
            status,
            html: t.render()?,
        })
    }

    /// Converts into a response with extra htmx headers attached.
    pub(crate) fn with(self, headers: HxHeaders) -> Response {
        let mut res = self.into_response();
        headers.apply_to(res.headers_mut());
        res
    }
}

impl IntoResponse for HtmxResponse {
    fn into_response(self) -> Response {
        let header_only = |name, value: &str| {
            let mut r = StatusCode::OK.into_response();
            header::set(r.headers_mut(), name, value);
            r
        };

        let mut res = match self {
            Self::Full(html) | Self::Partial(html) => Html(html).into_response(),
            Self::Partials(items) => {
                Html(items.iter().map(PartialSwap::render).collect::<String>()).into_response()
            }
            Self::NoContent => StatusCode::NO_CONTENT.into_response(),
            Self::Redirect(url) => header_only(name::REDIRECT, &url),
            Self::Location(url) => header_only(name::LOCATION, &url),
            Self::Refresh => header_only(name::REFRESH, "true"),
            Self::Error { status, html } => (status, Html(html)).into_response(),
        };

        header::set_vary(res.headers_mut());
        res
    }
}
