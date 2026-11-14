//! Cookie parsing and the session cookie.

use http::{HeaderMap, HeaderValue, header};
use secrecy::ExposeSecret as _;

use crate::{
    error::AppError,
    identity::{SESSION_TTL, SessionToken},
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
/// of CSRF protection (the htmx header check in `AppForm` is the second).
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
    pub(crate) fn issue(token: &SessionToken, secure: bool) -> Result<HeaderValue, AppError> {
        Self::header(
            token.expose_secret(),
            crate::util::time::duration_ms(SESSION_TTL) / 1000,
            secure,
        )
    }

    /// `Set-Cookie` value that deletes the cookie in the browser.
    pub(crate) fn clear(secure: bool) -> HeaderValue {
        let secure = if secure { "; Secure" } else { "" };
        HeaderValue::try_from(format!(
            "sid=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}"
        ))
        .unwrap_or_else(|_| HeaderValue::from_static("sid=; Path=/; Max-Age=0"))
    }

    fn header(value: &str, max_age_secs: i64, secure: bool) -> Result<HeaderValue, AppError> {
        let secure = if secure { "; Secure" } else { "" };
        HeaderValue::try_from(format!(
            "{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure}",
            Self::KEY,
        ))
        .map_err(AppError::internal)
    }
}
