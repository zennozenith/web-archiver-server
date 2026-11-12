use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use http::{header::USER_AGENT, request::Parts};

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

    pub(crate) fn extract(parts: &Parts) -> Result<Self, AppError> {
        // TODO:
        // Settings
        // pub trusted_proxy: bool,

        // // extractor: only read the header when the connection came from your proxy
        // let ip = if state.trusted_proxy {
        //     forwarded_for(&parts.headers).unwrap_or(addr.ip())
        // } else {
        //     addr.ip()
        // };
        //
        // axum-client-ip crate

        let ConnectInfo(addr) = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .copied()
            .ok_or_else(|| AppError::Internal("missing ConnectInfo".into()))?;

        let ua = parts.headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
        Ok(Self::new(addr.ip(), ua))
    }
}
