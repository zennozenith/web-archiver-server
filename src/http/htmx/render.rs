//! Plain askama rendering.

use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

/// Renders an askama template as an HTML response.
///
/// A render failure is logged and answered with a bare 500.
#[derive(Debug)]
pub(crate) struct Render<T>(pub(crate) T);

impl<T: Template> IntoResponse for Render<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(e) => {
                tracing::error!("template render failed: {e}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}
