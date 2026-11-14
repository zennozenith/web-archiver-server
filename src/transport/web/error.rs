//! Inline-alert errors for htmx form submissions (login, register, ...).

use axum::{
    extract::Request,
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use super::alert::Alert;
use crate::{
    error::{AppError, ErrorContext, ForbiddenReason},
    transport::htmx::HtmxRequest,
};

/// Layer for routes that answer with an inline form alert.
///
/// * Client errors (4xx) become an error [`Alert`] with status **422**, even
///   a 401 such as "invalid credentials". htmx must swap the alert in, and a
///   401 here means "wrong input", not "go sign in".
/// * Server errors (5xx) pass through untouched, so the outer
///   `htmx::render_errors` layer shows a toast with a generic message.
pub(super) async fn inline_alerts(req: Request, next: Next) -> Response {
    let res = next.run(req).await;

    let message = res
        .extensions()
        .get::<ErrorContext>()
        .filter(|info| info.status.is_client_error())
        .map(|info| info.message.clone());

    message.map_or(res, |message| {
        Alert::error(message).respond(StatusCode::UNPROCESSABLE_ENTITY)
    })
}

/// CSRF guard for the whole HTML router.
///
/// A cross-site HTML form cannot set `HX-Request`, which complements
/// `SameSite=Lax`. Covers every state-changing route, including pre-auth
/// ones (login CSRF).
pub(crate) async fn require_htmx_for_writes(req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && !HtmxRequest::from_headers(req.headers()).is_htmx {
        return AppError::Forbidden(ForbiddenReason::Csrf).into_response();
    }
    next.run(req).await
}
