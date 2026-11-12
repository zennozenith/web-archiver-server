use askama::Template;
use axum::{
    http::{HeaderValue, StatusCode, header::HeaderName},
    response::{Html, IntoResponse, Response},
};

use crate::prelude::*;

const HX_REQUEST: &str = "hx-request";

// ------------------------------------------------------------------ rendering

/// Renders an askama template as an HTML response.
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

/// Tell htmx to navigate (full page load) after a successful request.
pub(crate) fn with_redirect(mut resp: Response, to: &'static str) -> Response {
    resp.headers_mut().insert(
        HeaderName::from_static("hx-redirect"),
        HeaderValue::from_static(to),
    );
    resp
}

// --------------------------------------------------------------------- errors

#[derive(Debug, Template)]
#[template(path = "error.templ.html")]
struct ErrorTpl {
    title: &'static str,
    code: u16,
    message: String,
}

/// Error for full-page routes: renders a standalone error page.
#[derive(Debug)]
pub(crate) struct PageError(AppError);

impl From<AppError> for PageError {
    fn from(e: AppError) -> Self {
        Self(e)
    }
}

impl From<sqlx::Error> for PageError {
    fn from(e: sqlx::Error) -> Self {
        Self(AppError::Database(e))
    }
}

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        let (status, ..) = self.0.parts();
        let page = ErrorTpl {
            title: "Something went wrong",
            code: status.as_u16(),
            message: self.0.to_string(),
        };
        (status, Render(page)).into_response()
    }
}

/// Error for htmx action/fragment routes: answers with an error toast and
/// tells htmx not to touch the main target.
#[derive(Debug)]
pub(crate) struct ActionError(AppError);

impl From<AppError> for ActionError {
    fn from(e: AppError) -> Self {
        Self(e)
    }
}

impl From<sqlx::Error> for ActionError {
    fn from(e: sqlx::Error) -> Self {
        Self(AppError::Database(e))
    }
}

fn unauthorized_redirect() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            HeaderName::from_static("hx-redirect"),
            HeaderValue::from_static("/login"),
        )],
    )
        .into_response()
}

impl IntoResponse for ActionError {
    fn into_response(self) -> Response {
        let (status, ..) = self.0.parts();
        if status == StatusCode::UNAUTHORIZED {
            return unauthorized_redirect();
        }
        super::Fragments::new()
            .status(status)
            .header("hx-reswap", "none")
            .toast(super::ToastKind::Error, &self.0.to_string())
            .map_or_else(IntoResponse::into_response, IntoResponse::into_response)
    }
}
