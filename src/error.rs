use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Failed to render template. Error: {0}")]
    Render(#[from] askama::Error),
    #[error("an error occurred: {0}")]
    StringError(String),
    #[error("Something went wrong: {0}")]
    AnyHow(#[from] anyhow::Error),
}
impl From<&str> for AppError {
    fn from(value: &str) -> Self {
        AppError::StringError(value.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        #[derive(Template)]
        #[template(path = "error.html")]
        struct Tmpl<'a> {
            title: &'a str,
            msg: &'a str,
        }

        let (status, msg) = match &self {
            AppError::Render(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            AppError::AnyHow(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            AppError::StringError(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_owned()),
        };
        tracing::error!(msg);
        let tmpl = Tmpl {
            msg: &msg,
            title: "Error",
        };
        if let Ok(body) = tmpl.render() {
            (status, Html(body)).into_response()
        } else {
            (status, "Something went wrong").into_response()
        }
    }
}
