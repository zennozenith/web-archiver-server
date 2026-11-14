//! Renders `AppError` for the HTML side of the app.
//!
//! Handlers and extractors just return `AppError`. `AppError::into_response`
//! logs server errors once and attaches a sanitized `ErrorContext`; this layer
//! turns that into a toast, an error page, or leaves the JSON alone,
//! depending on what the request is.

use askama::Template;
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode, header as http_header},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};

use super::{Fragments, HtmxRequest, HxHeaders, Render, ToastKind, header, with_redirect};
use crate::{error::ErrorContext, http::LOGIN_PATH};

/// Shown instead of the sanitized-but-terse message for server errors.
const GENERIC_SERVER_MESSAGE: &str = "An unexpected error occurred. Please try again.";

#[derive(Debug, Template)]
#[template(path = "error.templ.html")]
struct ErrorTpl {
    title: &'static str,
    code: u16,
    message: String,
}

/// How the client wants to be told about a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    /// htmx action or fragment request: toast, leave the page untouched.
    Toast,
    /// Boosted link/form or history restore: htmx expects a whole page.
    HxPage,
    /// Plain browser navigation: error page, real HTTP redirects.
    Page,
    /// Not an HTML client: keep the JSON body.
    Json,
}

impl Surface {
    fn detect(headers: &HeaderMap) -> Self {
        let hx = HtmxRequest::from_headers(headers);
        if hx.is_htmx && !hx.boosted && !hx.history_restore {
            Self::Toast
        } else if hx.is_htmx {
            Self::HxPage
        } else if accepts_html(headers) {
            Self::Page
        } else {
            Self::Json
        }
    }
}

fn accepts_html(headers: &HeaderMap) -> bool {
    header::get(headers, http_header::ACCEPT.as_str()).is_some_and(|v| v.contains("text/html"))
}

/// Middleware for the HTML router only, so `/api` keeps pure JSON.
pub(crate) async fn render_errors(req: Request, next: Next) -> Response {
    let surface = Surface::detect(req.headers());
    let mut res = next.run(req).await;

    let Some(info) = res.extensions_mut().remove::<ErrorContext>() else {
        return res;
    };

    if surface == Surface::Json {
        res.extensions_mut().insert(info);
        return res;
    }

    let mut out = if info.status == StatusCode::UNAUTHORIZED {
        login_redirect(surface)
    } else if surface == Surface::Toast {
        toast_response(&info)
    } else {
        page_response(&info)
    };
    header::set_vary(out.headers_mut());
    out
}

fn login_redirect(surface: Surface) -> Response {
    match surface {
        Surface::Toast | Surface::HxPage => {
            with_redirect(StatusCode::UNAUTHORIZED.into_response(), LOGIN_PATH)
        }
        Surface::Page | Surface::Json => Redirect::to(LOGIN_PATH).into_response(),
    }
}

fn display_message(info: &ErrorContext) -> String {
    if info.status.is_server_error() {
        GENERIC_SERVER_MESSAGE.to_owned()
    } else {
        info.message.clone()
    }
}

fn toast_response(info: &ErrorContext) -> Response {
    let message = display_message(info);
    Fragments::new()
        .status(info.status)
        .hx(HxHeaders::new().reswap("none"))
        .toast(ToastKind::Error, &message)
        .map_or_else(
            |_| (info.status, message).into_response(),
            IntoResponse::into_response,
        )
}

fn page_response(info: &ErrorContext) -> Response {
    let page = ErrorTpl {
        title: title_for(info.status),
        code: info.status.as_u16(),
        message: display_message(info),
    };
    (info.status, Render(page)).into_response()
}

fn title_for(status: StatusCode) -> &'static str {
    match status {
        StatusCode::NOT_FOUND => "Page not found",
        StatusCode::FORBIDDEN => "Access denied",
        s if s.is_server_error() => "Something went wrong",
        _ => "Request failed",
    }
}
