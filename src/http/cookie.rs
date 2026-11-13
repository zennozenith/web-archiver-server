//! Cookie parsing and the session cookie.

use http::{HeaderMap, HeaderValue, header};
use secrecy::ExposeSecret as _;

use crate::{
    auth::{SESSION_TTL_MS, SessionToken},
    error::AppError,
};

/// Returns the value of the first cookie called `name`, across all `Cookie`
/// headers.
pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .find_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            (k == name).then(|| v.to_owned())
        })
}

/// The `sid` cookie that carries the session token.
///
/// `HttpOnly` keeps it away from scripts, and `SameSite=Lax` is the base layer
/// of CSRF protection (the htmx header check in `WebForm` is the second).
pub(crate) struct SessionCookie;

impl SessionCookie {
    pub(crate) const KEY: &'static str = "sid";

    /// The session token from the request's cookies, if present and well-formed.
    pub(crate) fn get(headers: &HeaderMap) -> Option<SessionToken> {
        cookie_value(headers, Self::KEY).and_then(|v| SessionToken::try_from(v).ok())
    }

    /// `Set-Cookie` value that stores `token` for the session lifetime.
    ///
    /// # Errors
    ///
    /// Returns an internal error if the token contains characters that are
    /// not valid in a header. Generated tokens never do, so this indicates a
    /// bug rather than bad input.
    pub(crate) fn issue(token: &SessionToken) -> Result<HeaderValue, AppError> {
        Self::header(token.expose_secret(), SESSION_TTL_MS / 1000)
    }

    /// `Set-Cookie` value that deletes the cookie in the browser.
    pub(crate) const fn clear() -> HeaderValue {
        HeaderValue::from_static("sid=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
    }

    // TODO: add `; Secure` when serving over HTTPS (make it a config flag).
    fn header(value: &str, max_age_secs: i64) -> Result<HeaderValue, AppError> {
        HeaderValue::try_from(format!(
            "{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}",
            Self::KEY,
        ))
        .map_err(|e| AppError::Internal(Box::new(e)))
    }
}
