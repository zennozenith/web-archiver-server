use http::{HeaderMap, header};
use secrecy::ExposeSecret as _;

use crate::auth::{SESSION_TTL_MS, SessionToken};

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

pub(crate) struct SessionCookie;

impl SessionCookie {
    pub(crate) const KEY: &str = "sid";

    /// Add `; Secure` when serving over HTTPS (make it a config flag).
    pub(crate) fn set(token: &SessionToken) -> String {
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
            Self::KEY,
            token.expose_secret(),
            SESSION_TTL_MS / 1000
        )
    }

    pub(crate) fn clear() -> String {
        format!("{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0", Self::KEY)
    }

    pub(crate) fn get(headers: &HeaderMap) -> Option<SessionToken> {
        cookie_value(headers, Self::KEY).and_then(|v| SessionToken::try_from(v).ok())
    }
}
