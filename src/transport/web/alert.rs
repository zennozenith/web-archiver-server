//! The inline alert that htmx swaps into `#form-alert`.

use askama::Template;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::transport::htmx::Fragments;

/// Feedback shown under a form, optionally followed by a link.
///
/// An empty `href` hides the link in the template.
#[derive(Debug, Template)]
#[template(path = "partials/alert.templ.html")]
pub(super) struct Alert {
    kind: &'static str,
    message: String,
    href: &'static str,
    label: &'static str,
}

impl Alert {
    pub(super) fn success(message: impl Into<String>) -> Self {
        Self::new("alert-success", message)
    }

    pub(super) fn error(message: impl Into<String>) -> Self {
        Self::new("alert-error", message)
    }

    /// Adds a call-to-action link, e.g. "Sign in".
    pub(super) const fn link(mut self, href: &'static str, label: &'static str) -> Self {
        self.href = href;
        self.label = label;
        self
    }

    /// Renders the alert with an explicit HTTP status.
    pub(super) fn respond(self, status: StatusCode) -> Response {
        Fragments::new()
            .status(status)
            .push(&self)
            // If the alert itself fails to render, fall back to the plain error.
            .map_or_else(IntoResponse::into_response, IntoResponse::into_response)
    }

    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            href: "",
            label: "",
        }
    }
}

impl IntoResponse for Alert {
    fn into_response(self) -> Response {
        self.respond(StatusCode::OK)
    }
}
