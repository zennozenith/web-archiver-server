use askama::Template;
use axum::response::Html;

use crate::error::AppError;

pub trait RenderHtml {
    fn render_html(&self) -> Result<Html<String>, AppError>;
}

impl<T: Template> RenderHtml for T {
    fn render_html(&self) -> Result<Html<String>, AppError> {
        self.render().map(Html).map_err(AppError::Render)
    }
}
