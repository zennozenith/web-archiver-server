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
