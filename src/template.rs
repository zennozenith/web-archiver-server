use askama::Template;
use axum::response::Html;

use crate::error::{AppError, Result};

pub trait RenderHtml {
    fn render_html(&self) -> Result<Html<String>>;
}

impl<T: Template> RenderHtml for T {
    fn render_html(&self) -> Result<Html<String>> {
        self.render().map(Html).map_err(AppError::from)
    }
}

#[derive(Clone, Copy)]
pub enum FeedbackKind {
    Success,
    Error,
    Warning,
}

impl FeedbackKind {
    /// DaisyUI alert modifier class.
    pub fn class(&self) -> &'static str {
        match self {
            Self::Success => "alert-success",
            Self::Error => "alert-error",
            Self::Warning => "alert-warning",
        }
    }
}

#[derive(Template)]
#[template(path = "fragments/feedback.html")]
pub struct FeedbackTemplate<'a> {
    pub kind: FeedbackKind,
    pub message: &'a str,
}

impl<'a> FeedbackTemplate<'a> {
    pub fn success(message: &'a str) -> Self {
        Self {
            kind: FeedbackKind::Success,
            message,
        }
    }
    pub fn error(message: &'a str) -> Self {
        Self {
            kind: FeedbackKind::Error,
            message,
        }
    }
    pub fn warning(message: &'a str) -> Self {
        Self {
            kind: FeedbackKind::Warning,
            message,
        }
    }
}
