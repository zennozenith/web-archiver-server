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

pub trait IntoAnyhow<T> {
    fn anyhow(self) -> anyhow::Result<T>;
}

impl<T, E> IntoAnyhow<T> for Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn anyhow(self) -> anyhow::Result<T> {
        self.map_err(anyhow::Error::from)
    }
}
