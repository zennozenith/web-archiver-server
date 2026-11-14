use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use http::{HeaderMap, header::USER_AGENT, request::Parts};

use crate::{app::ServerConfig, error::AppError, util::net::Cidr};

/// Who is calling, as far as the server can tell. Stored in the login audit
/// trail and on sessions.
#[derive(Debug, Clone)]
pub(crate) struct ClientInfo {
    pub ip: IpAddr,
    pub user_agent: String,
}

/// UAs are attacker-controlled and get stored, so cap the length.
const MAX_UA_LEN: usize = 256;

/// The only forwarding header the server reads. One header, configured once,
/// is easier to reason about than a precedence list a client can influence.
const FORWARDED_FOR: &str = "x-forwarded-for";

impl ClientInfo {
    pub(crate) fn new(ip: IpAddr, user_agent_any: Option<&str>) -> Self {
        let ua = user_agent_any.unwrap_or("unknown");
        let user_agent = ua.chars().take(MAX_UA_LEN).collect();
        Self { ip, user_agent }
    }

    pub(crate) fn extract(parts: &Parts, server: &ServerConfig) -> Result<Self, AppError> {
        let ConnectInfo(addr) = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .copied()
            .ok_or_else(|| AppError::internal("missing ConnectInfo"))?;

        // Only trust forwarding headers when the socket peer itself is a
        // configured proxy. A client connecting directly can forge the header,
        // but its peer address will never match `trusted_proxies`.
        let ip = if is_trusted_proxy(addr.ip(), &server.trusted_proxies) {
            forwarded_ip(&parts.headers, &server.trusted_proxies).unwrap_or_else(|| addr.ip())
        } else {
            addr.ip()
        };

        let ua = parts.headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
        Ok(Self::new(ip, ua))
    }
}

fn is_trusted_proxy(ip: IpAddr, trusted_proxies: &[Cidr]) -> bool {
    trusted_proxies.iter().any(|cidr| cidr.contains(ip))
}

/// The client address from `X-Forwarded-For`.
///
/// Proxies append the address of the peer they received the request from, so
/// the rightmost entries are the trustworthy ones. Walking from right to left
/// and skipping trusted proxies yields the first hop we did not put there
/// ourselves. A client-supplied prefix (the spoofing vector) is never reached
/// as long as at least one proxy appended the real peer.
fn forwarded_ip(headers: &HeaderMap, trusted_proxies: &[Cidr]) -> Option<IpAddr> {
    let hops: Vec<IpAddr> = headers
        .get_all(FORWARDED_FOR)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|v| parse_forwarded_addr(v.trim()))
        .collect();

    let mut fallback = None;
    for hop in hops.into_iter().rev() {
        if !is_trusted_proxy(hop, trusted_proxies) {
            return Some(hop);
        }
        fallback = Some(hop);
    }
    fallback
}

/// Parse an X-Forwarded-For entry. IPv6 addresses may be bracketed and may
/// carry a port (`[::1]:1234`); IPv4 may carry a port (`203.0.113.7:80`).
fn parse_forwarded_addr(value: &str) -> Option<IpAddr> {
    if let Ok(ip) = value.parse() {
        return Some(ip);
    }
    if let Some(inner) = value.strip_prefix('[') {
        let (ip, _) = inner.split_once(']')?;
        return ip.parse().ok();
    }
    let (ip, port) = value.rsplit_once(':')?;
    if port.chars().all(|c| c.is_ascii_digit()) {
        ip.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderValue, header::FORWARDED};

    fn cidrs(values: &[&str]) -> Vec<Cidr> {
        values
            .iter()
            .map(|v| v.parse().expect("valid cidr"))
            .collect()
    }

    #[test]
    fn rightmost_hop_wins_over_client_supplied_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert(
            FORWARDED_FOR,
            HeaderValue::from_static("1.2.3.4, 203.0.113.7"),
        );
        assert_eq!(
            forwarded_ip(&headers, &cidrs(&[])),
            Some("203.0.113.7".parse().expect("ip"))
        );
    }

    #[test]
    fn trusted_proxy_hops_are_skipped_from_the_right() {
        let mut headers = HeaderMap::new();
        headers.insert(
            FORWARDED_FOR,
            HeaderValue::from_static("198.51.100.9, 10.0.0.2, 10.0.0.3"),
        );
        assert_eq!(
            forwarded_ip(&headers, &cidrs(&["10.0.0.0/8"])),
            Some("198.51.100.9".parse().expect("ip"))
        );
    }

    #[test]
    fn forwarded_header_cannot_override_x_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert(FORWARDED, HeaderValue::from_static("for=1.2.3.4"));
        headers.insert(FORWARDED_FOR, HeaderValue::from_static("203.0.113.7"));
        assert_eq!(
            forwarded_ip(&headers, &cidrs(&[])),
            Some("203.0.113.7".parse().expect("ip"))
        );
    }

    #[test]
    fn bracketed_and_ported_entries_are_parsed() {
        let mut headers = HeaderMap::new();
        headers.insert(
            FORWARDED_FOR,
            HeaderValue::from_static("[2001:db8::1]:443, 203.0.113.7:8080"),
        );
        assert_eq!(
            forwarded_ip(&headers, &cidrs(&[])),
            Some("203.0.113.7".parse().expect("ip"))
        );
    }
}
