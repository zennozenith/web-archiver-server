use axum::{
    extract::{FromRef, FromRequestParts},
    http::{Method, request::Parts},
    response::{IntoResponse, Redirect, Response},
};
use sqlx::SqlitePool;

use crate::{
    auth::AuthUser,
    error::{AppError, ForbiddenReason},
    http::htmx::HtmxRequest,
};

/// A signed-in user for HTML routes.
///
/// State-changing requests must come from htmx. A cross-site HTML form can't
/// set `HX-Request`, which complements `SameSite=Lax` as CSRF protection.
#[derive(Debug)]
pub(crate) struct WebUser(pub(crate) AuthUser);

pub(crate) struct WebRejection(AppError, bool /* htmx */);

impl IntoResponse for WebRejection {
    fn into_response(self) -> Response {
        match self {
            // Unauthorized is turned into a redirect by `negotiate_errors`;
            // everything else is rendered by it too. Just emit the AppError.
            Self(e, _) => e.into_response(),
        }
    }
}

impl<S> FromRequestParts<S> for WebUser
where
    S: Send + Sync,
    SqlitePool: FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let htmx = HtmxRequest::from_headers(&parts.headers).is_htmx;
        let safe = matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS);
        if !safe && !htmx {
            return Err(AppError::Forbidden(ForbiddenReason::Csrf));
        }
        AuthUser::from_request_parts(parts, state).await.map(Self)
    }
}
