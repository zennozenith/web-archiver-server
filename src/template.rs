#![expect(
    dead_code,
    reason = "feedback variants cover success/error/warning for future handlers"
)]

use askama::Template;
use axum::response::Html;

use crate::error::{AppError, Result};

pub(crate) trait RenderHtml {
    fn render_html(&self) -> Result<Html<String>>;
}

impl<T: Template> RenderHtml for T {
    fn render_html(&self) -> Result<Html<String>> {
        self.render().map(Html).map_err(AppError::from)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum FeedbackKind {
    Success,
    Error,
    Warning,
}

impl FeedbackKind {
    /// `DaisyUI` alert modifier class.
    pub(crate) const fn class(self) -> &'static str {
        match self {
            Self::Success => "alert-success",
            Self::Error => "alert-error",
            Self::Warning => "alert-warning",
        }
    }
}

#[derive(Template)]
#[template(path = "fragments/feedback.html")]
pub(crate) struct FeedbackTemplate<'a> {
    pub kind: FeedbackKind,
    pub message: &'a str,
}

impl<'a> FeedbackTemplate<'a> {
    pub(crate) const fn success(message: &'a str) -> Self {
        Self {
            kind: FeedbackKind::Success,
            message,
        }
    }
    pub(crate) const fn error(message: &'a str) -> Self {
        Self {
            kind: FeedbackKind::Error,
            message,
        }
    }
    pub(crate) const fn warning(message: &'a str) -> Self {
        Self {
            kind: FeedbackKind::Warning,
            message,
        }
    }
}
