// src/http/web/error.rs
use askama::Template;
use axum::{
    extract::Request,
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use http::{HeaderValue, StatusCode, header::ACCEPT};

use crate::{
    error::ErrorInfo,
    http::htmx::{Fragments, HtmxRequest, ToastKind},
};

#[derive(Template)]
#[template(path = "error.templ.html")]
struct ErrorPage<'a> {
    title: &'a str,
    code: u16,
    message: &'a str,
}

/// Re-renders `AppError` responses for browsers. Handlers and services just
/// return `AppError`; this decides what the client sees.
///
/// | request              | 401                            | other errors                   |
/// |----------------------|--------------------------------|--------------------------------|
/// | htmx                 | 401 + `HX-Redirect: /login`    | error toast, `HX-Reswap: none` |
/// | browser (text/html)  | 303 to `/login`                | full error page                |
/// | anything else        | JSON unchanged                 | JSON unchanged                 |
pub(crate) async fn negotiate_errors(htmx: HtmxRequest, req: Request, next: Next) -> Response {
    let wants_html = req
        .headers()
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("text/html"));

    let mut res = next.run(req).await;
    let Some(info) = res.extensions_mut().remove::<ErrorInfo>() else {
        return res;
    };
    let unauthorized = info.status == StatusCode::UNAUTHORIZED;

    if htmx.is_htmx {
        if unauthorized {
            let mut r = StatusCode::UNAUTHORIZED.into_response();
            r.headers_mut()
                .insert("hx-redirect", HeaderValue::from_static("/login"));
            return r;
        }
        return Fragments::new()
            .status(info.status)
            .header("hx-reswap", "none")
            .toast(ToastKind::Error, &info.message)
            .map_or_else(IntoResponse::into_response, IntoResponse::into_response);
    }

    if wants_html {
        if unauthorized {
            return Redirect::to("/login").into_response();
        }
        let page = ErrorPage {
            title: "Something went wrong",
            code: info.status.as_u16(),
            message: &info.message,
        };
        return match page.render() {
            Ok(html) => (info.status, Html(html)).into_response(),
            Err(_) => info.status.into_response(),
        };
    }

    res
}
