//! Form extraction for htmx-only POST routes.

use axum::{
    Form,
    extract::{FromRequest, Request},
    http::HeaderMap,
};
use serde::de::DeserializeOwned;

use super::error::WebError;
use crate::{
    error::{AppError, ForbiddenReason},
    http::htmx::{ActionError, HtmxRequest},
};

/// Rejects requests that did not come from htmx.
///
/// A cross-site HTML form cannot set `HX-Request`, which complements
/// `SameSite=Lax` as CSRF protection. Applied to every state-changing route,
/// including the pre-auth ones (login CSRF).
pub(super) fn require_htmx(headers: &HeaderMap) -> Result<(), AppError> {
    if HtmxRequest::from_headers(headers).is_htmx {
        Ok(())
    } else {
        Err(AppError::Forbidden(ForbiddenReason::Csrf))
    }
}

/// An urlencoded form body from an htmx request. Failures render as a
/// [`WebError`] instead of axum's plain-text rejection.
#[derive(Debug)]
pub(super) struct WebForm<T>(pub T);

impl<S, T> FromRequest<S> for WebForm<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = WebError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        require_htmx(req.headers())?;
        let Form(value) = Form::<T>::from_request(req, state).await?;
        Ok(Self(value))
    }
}

/// Urlencoded body for htmx action routes. Failures become a toast.
#[derive(Debug, FromRequest)]
#[from_request(via(axum::Form), rejection(ActionError))]
pub(super) struct ActionForm<T>(pub T);
