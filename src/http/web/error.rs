//! Error type for htmx form submissions (login, register, ...).

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::alert::Alert;
use crate::{error::AppError, http::htmx::ActionError};

/// Error for routes that answer with an inline form alert.
///
/// * Client errors (4xx) become an error [`Alert`] with status **422**, even
///   a 401 such as "invalid credentials". htmx must swap the alert in, and a
///   401 here means "wrong input", not "go sign in".
/// * Server errors (5xx) are handed to [`ActionError`]: a toast with a generic
///   message, `HX-Reswap: none`, and the real status.
///
/// Anything convertible to [`AppError`] (service errors, `sqlx::Error`, body
/// rejections) converts via `?`.
#[derive(Debug)]
pub(crate) struct WebError(AppError);

impl<E: Into<AppError>> From<E> for WebError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let (status, ..) = self.0.parts();
        if status.is_server_error() {
            return ActionError::from(self.0).into_response();
        }
        Alert::error(self.0.public_message(status)).respond(StatusCode::UNPROCESSABLE_ENTITY)
    }
}
