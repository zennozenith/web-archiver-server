use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use http::{
    HeaderMap,
    header::{FORWARDED, USER_AGENT},
    request::Parts,
};

use crate::error::AppError;

/// Who is calling, as far as the server can tell. Stored in the login audit
/// trail and on sessions.
#[derive(Debug, Clone)]
pub(crate) struct ClientInfo {
    pub ip: IpAddr,
    pub user_agent: String,
}

/// UAs are attacker-controlled and get stored, so cap the length.
const MAX_UA_LEN: usize = 256;

impl ClientInfo {
    pub(crate) fn new(ip: IpAddr, user_agent_any: Option<&str>) -> Self {
        let ua = user_agent_any.unwrap_or("unknown");
        let user_agent = ua.chars().take(MAX_UA_LEN).collect();
        Self { ip, user_agent }
    }

    pub(crate) fn extract(parts: &Parts, trusted_proxy: bool) -> Result<Self, AppError> {
        let ConnectInfo(addr) = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .copied()
            .ok_or_else(|| AppError::internal("missing ConnectInfo"))?;

        // Only trust forwarding headers when the socket peer is the proxy.
        let ip = if trusted_proxy {
            forwarded_ip(&parts.headers).unwrap_or_else(|| addr.ip())
        } else {
            addr.ip()
        };

        let ua = parts.headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
        Ok(Self::new(ip, ua))
    }
}

/// First usable client address from `Forwarded`, `X-Forwarded-For`, or
/// `X-Real-IP`.
fn forwarded_ip(headers: &HeaderMap) -> Option<IpAddr> {
    if let Some(ip) = headers
        .get_all(FORWARDED)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .find_map(forwarded_for_param)
    {
        return Some(ip);
    }

    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.trim().parse().ok())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse().ok())
        })
}

fn forwarded_for_param(entry: &str) -> Option<IpAddr> {
    entry.split(';').find_map(|param| {
        let (name, value) = param.trim().split_once('=')?;
        if name.trim().eq_ignore_ascii_case("for") {
            let value = value.trim().trim_matches('"');
            value
                .strip_prefix('[')
                .and_then(|v| v.split_once(']').map(|(ip, _)| ip))
                .unwrap_or(value)
                .parse()
                .ok()
        } else {
            None
        }
    })
}
