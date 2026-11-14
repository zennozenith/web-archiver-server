//! Responses assembled from several rendered templates.

use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

use super::{HxHeaders, header};
use crate::error::AppError;

/// Visual style of a toast notification.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ToastKind {
    Success,
    Error,
}

impl ToastKind {
    /// CSS class applied to the toast.
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

/// Builds one response out of several rendered templates: the main fragment,
/// out-of-band updates and toasts.
///
/// Order matters: push the main content first, then out-of-band parts.
#[derive(Debug, Default)]
pub(crate) struct Fragments {
    body: String,
    hx: HxHeaders,
    status: Option<StatusCode>,
}

impl Fragments {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Renders `tpl` and appends it to the body.
    ///
    /// # Errors
    ///
    /// Returns an error if the template fails to render.
    pub(crate) fn push<T: Template>(mut self, tpl: &T) -> Result<Self, AppError> {
        self.body.push_str(&tpl.render()?);
        Ok(self)
    }

    /// Appends a toast. It is swapped out-of-band into `#toasts`.
    ///
    /// # Errors
    ///
    /// Returns an error if the toast template fails to render.
    pub(crate) fn toast(self, kind: ToastKind, message: &str) -> Result<Self, AppError> {
        self.push(&ToastTpl {
            kind: kind.class(),
            message: message.to_owned(),
        })
    }

    /// Attaches htmx response headers.
    pub(crate) fn hx(mut self, headers: HxHeaders) -> Self {
        self.hx = headers;
        self
    }

    /// Sets the HTTP status (defaults to 200).
    pub(crate) const fn status(mut self, status: StatusCode) -> Self {
        self.status = Some(status);
        self
    }
}

impl IntoResponse for Fragments {
    fn into_response(self) -> Response {
        let mut res = (self.status.unwrap_or(StatusCode::OK), Html(self.body)).into_response();
        self.hx.apply_to(res.headers_mut());
        header::set_vary(res.headers_mut());
        res
    }
}
