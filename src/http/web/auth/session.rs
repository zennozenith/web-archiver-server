//! Redirect plumbing for sign-in and sign-out.

use axum::{
    http::{HeaderValue, header::SET_COOKIE},
    response::{IntoResponse, Response},
};

use super::HOME;
use crate::{
    auth::SessionToken,
    error::AppError,
    http::{cookie::SessionCookie, htmx::HtmxResponse},
};

/// `HX-Redirect` home, carrying the new session cookie.
pub(super) fn sign_in(token: &SessionToken) -> Result<Response, AppError> {
    Ok(redirect_with_cookie(HOME, SessionCookie::issue(token)?))
}

/// `HX-Redirect` to `to`, clearing the session cookie.
pub(super) fn sign_out(to: &str) -> Response {
    redirect_with_cookie(to, SessionCookie::clear())
}

fn redirect_with_cookie(to: &str, cookie: HeaderValue) -> Response {
    let mut res = HtmxResponse::Redirect(to.to_owned()).into_response();
    res.headers_mut().append(SET_COOKIE, cookie);
    res
}
