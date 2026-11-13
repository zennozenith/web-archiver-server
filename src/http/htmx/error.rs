//! Error types for the two kinds of route: full pages and htmx actions.

use askama::Template;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{Fragments, HxHeaders, Render, ToastKind, with_redirect};
use crate::prelude::*;

/// Where unauthenticated htmx requests are sent.
const LOGIN_PATH: &str = "/login";

/// Message shown instead of internal details for server errors.
const GENERIC_SERVER_MESSAGE: &str = "An unexpected error occurred. Please try again.";

/// Message safe to show the user. Server errors are logged and replaced with
/// a generic message so internals (SQL, paths) never leak.
fn public_message(err: &AppError, status: StatusCode) -> String {
    if status.is_server_error() {
        tracing::error!(error = %err, %status, "request failed");
        GENERIC_SERVER_MESSAGE.to_owned()
    } else {
        err.to_string()
    }
}

macro_rules! impl_error_from {
    ($ty:ident) => {
        impl From<AppError> for $ty {
            fn from(e: AppError) -> Self {
                Self(e)
            }
        }

        impl From<sqlx::Error> for $ty {
            fn from(e: sqlx::Error) -> Self {
                Self(AppError::Database(e))
            }
        }
    };
}

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
impl_error_from!(PageError);

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        let (status, ..) = self.0.parts();
        let page = ErrorTpl {
            title: "Something went wrong",
            code: status.as_u16(),
            message: public_message(&self.0, status),
        };
        (status, Render(page)).into_response()
    }
}

/// Error for htmx action and fragment routes.
///
/// Answers with an error toast and `HX-Reswap: none` so the main target is
/// left untouched. A 401 instead redirects the browser to the login page.
#[derive(Debug)]
pub(crate) struct ActionError(AppError);
impl_error_from!(ActionError);

impl From<axum::extract::rejection::FormRejection> for ActionError {
    fn from(rej: axum::extract::rejection::FormRejection) -> Self {
        Self(AppError::from(rej))
    }
}

impl IntoResponse for ActionError {
    fn into_response(self) -> Response {
        let (status, ..) = self.0.parts();
        if status == StatusCode::UNAUTHORIZED {
            return with_redirect(status.into_response(), LOGIN_PATH);
        }

        Fragments::new()
            .status(status)
            .hx(HxHeaders::new().reswap("none"))
            .toast(ToastKind::Error, &public_message(&self.0, status))
            // If even the toast fails to render, fall back to the plain error.
            .map_or_else(IntoResponse::into_response, IntoResponse::into_response)
    }
}
