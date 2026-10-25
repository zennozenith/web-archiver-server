use askama::Template;
use axum::{
    extract::Request,
    middleware::Next,
    response::{Html, IntoResponse, Response},
};
use http::{HeaderMap, StatusCode, header};

use crate::error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wants {
    Json,
    Page,
    Fragment,
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage<'a> {
    status: u16,
    title: &'a str,
    message: &'a str,
}

#[derive(Template)]
#[template(path = "fragments/error.html")]
struct ErrorFragment<'a> {
    message: &'a str,
}

fn wants(headers: &HeaderMap) -> Wants {
    if headers.contains_key("hx-request") && !headers.contains_key("hx-boosted") {
        Wants::Fragment
    } else if prefers_html(headers) {
        Wants::Page
    } else {
        Wants::Json
    }
}

/// Returns true only if the client explicitly asks for HTML and ranks it
/// at least as high as JSON. `*/*` (curl, fetch defaults) stays JSON.
fn prefers_html(headers: &HeaderMap) -> bool {
    let Some(accept) = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()) else {
        return false;
    };

    let (mut html, mut json): (Option<f32>, Option<f32>) = (None, None);

    for part in accept.split(',') {
        let mut it = part.split(';');
        let mime = it.next().unwrap_or("").trim().to_ascii_lowercase();
        let q = it
            .find_map(|p| p.trim().strip_prefix("q=")?.parse::<f32>().ok())
            .unwrap_or(1.0);

        match mime.as_str() {
            "text/html" | "application/xhtml+xml" => {
                html = Some(html.map_or(q, |h| h.max(q)));
            }
            "application/json" => {
                json = Some(json.map_or(q, |j| j.max(q)));
            }
            _ => {}
        }
    }

    match (html, json) {
        (Some(h), Some(j)) => h > j,
        (Some(h), None) => h > 0.0,
        _ => false,
    }
}

pub async fn negotiate_errors(req: Request, next: Next) -> Response {
    let wants = wants(req.headers());
    let mut res = next.run(req).await;

    if matches!(wants, Wants::Json) {
        return res;
    }
    let Some(info) = res.extensions_mut().remove::<error::ErrorInfo>() else {
        return res;
    };

    let rendered = match wants {
        Wants::Fragment => ErrorFragment {
            message: &info.message,
        }
        .render(),
        _ => ErrorPage {
            status: info.status.as_u16(),
            title: info.status.canonical_reason().unwrap_or("Error"),
            message: &info.message,
        }
        .render(),
    };

    let mut out = match rendered {
        Ok(body) => (info.status, Html(body)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "failed to render error template");
            (info.status, "Something went wrong").into_response()
        }
    };

    // keep handler-set headers (WWW-Authenticate etc.)
    for (k, v) in res.headers() {
        if k != header::CONTENT_TYPE && k != header::CONTENT_LENGTH {
            out.headers_mut().insert(k.clone(), v.clone());
        }
    }

    if matches!(wants, Wants::Fragment) {
        let h = out.headers_mut();
        // Put the error in one global container instead of the original target.
        h.insert("HX-Retarget", "#error-container".parse().unwrap());
        h.insert("HX-Reswap", "innerHTML".parse().unwrap());
        // Optional: bounce expired sessions to login
        if info.status == StatusCode::UNAUTHORIZED {
            h.insert("HX-Redirect", "/login".parse().unwrap());
        }
    }

    // Same URL, different representation depending on these headers.
    out.headers_mut().append(
        header::VARY,
        "Accept, HX-Request, HX-Boosted".parse().unwrap(),
    );
    out
}
