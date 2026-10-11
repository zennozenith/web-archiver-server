use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum AppError {
    /// Failed to render template. Error: {0}
    Render(#[from] askama::Error),
    /// Something went wrong: {0}
    AnyHow(#[from] anyhow::Error),
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
        };
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
