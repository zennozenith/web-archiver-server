//! Header names and small helpers shared by the request and response sides.

use axum::http::{HeaderMap, HeaderName, HeaderValue};

/// Names of the htmx headers. Lowercase, as required by
/// [`HeaderName::from_static`].
pub(super) mod name {
    // Request
    pub(crate) const REQUEST: &str = "hx-request";
    pub(crate) const BOOSTED: &str = "hx-boosted";
    pub(crate) const HISTORY_RESTORE_REQUEST: &str = "hx-history-restore-request";
    pub(crate) const REQUEST_TYPE: &str = "hx-request-type";
    pub(crate) const TARGET: &str = "hx-target";
    pub(crate) const SOURCE: &str = "hx-source";
    pub(crate) const CURRENT_URL: &str = "hx-current-url";

    // Response
    pub(crate) const RETARGET: &str = "hx-retarget";
    pub(crate) const RESWAP: &str = "hx-reswap";
    pub(crate) const RESELECT: &str = "hx-reselect";
    pub(crate) const TRIGGER: &str = "hx-trigger";
    pub(crate) const TRIGGER_AFTER_SWAP: &str = "hx-trigger-after-swap";
    pub(crate) const TRIGGER_AFTER_SETTLE: &str = "hx-trigger-after-settle";
    pub(crate) const PUSH_URL: &str = "hx-push-url";
    pub(crate) const REPLACE_URL: &str = "hx-replace-url";
    pub(crate) const REDIRECT: &str = "hx-redirect";
    pub(crate) const LOCATION: &str = "hx-location";
    pub(crate) const REFRESH: &str = "hx-refresh";
}

/// Request headers that change the shape of the response.
const VARY: &str = "HX-Request, HX-Request-Type, HX-History-Restore-Request";

/// Returns a header value as `&str`, if present and valid UTF-8.
pub(super) fn get<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// True when the header is present with the exact value `true`.
pub(super) fn is_true(headers: &HeaderMap, name: &str) -> bool {
    get(headers, name) == Some("true")
}

/// Inserts a header, logging and skipping values that are not valid header
/// content (e.g. contain newlines).
pub(super) fn set(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        headers.insert(HeaderName::from_static(name), v);
    } else {
        tracing::warn!(header = name, "dropping invalid htmx header value");
    }
}

/// Marks a response as varying on the htmx request headers. Required whenever
/// the same URL can return either a full page or a fragment.
pub(super) fn set_vary(headers: &mut HeaderMap) {
    headers.insert(axum::http::header::VARY, HeaderValue::from_static(VARY));
}
