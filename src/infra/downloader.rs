//! HTTP fetching of page sub-resources.
//!
//! The original scratch implementation (see `examples/parallel_download.rs`)
//! wrote every response to disk and compressed it there. Archived content is
//! stored through `db::store`, which already compresses and deduplicates, so
//! this module only needs to fetch bytes and report the response metadata.
//!
//! Concurrency is handled by the caller with `futures::stream::buffer_unordered`.

use std::{net::IpAddr, sync::Arc};

use futures::StreamExt;
use reqwest::{
    dns::{Addrs, Name, Resolve, Resolving},
    header::CONTENT_TYPE,
};
use url::Url;

/// Hard ceiling per resource, independent of the request body limit.
pub(crate) const MAX_ASSET_BYTES: usize = 32 * 1024 * 1024;
/// Maximum redirects followed for one resource.
pub(crate) const MAX_REDIRECTS: usize = 10;

#[derive(Debug, thiserror::Error)]
pub(crate) enum FetchError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("blocked URL: {0}")]
    Blocked(String),
    #[error("http status {0}")]
    Status(u16),
    #[error("resource exceeds {0} bytes")]
    TooLarge(usize),
    #[error("empty response body")]
    Empty,
}

/// True only for globally routable unicast addresses. Everything special-use
/// (loopback, private, link-local, CGNAT, documentation, ...) is rejected.
pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.is_multicast()
                // 0.0.0.0/8 "this network"
                || octets[0] == 0
                // 100.64.0.0/10 carrier-grade NAT
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                // 192.0.0.0/24 IETF protocol assignments
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                // 198.18.0.0/15 benchmarking
                || (octets[0] == 198 && matches!(octets[1], 18 | 19))
                // 240.0.0.0/4 reserved
                || octets[0] >= 240)
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                // 2001:db8::/32 documentation
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

/// Rejects non-HTTP(S) schemes, literal non-public hosts, and localhost names.
pub(crate) fn validate_public_url(url: &Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http and https URLs may be fetched".to_owned());
    }
    match url.host() {
        Some(url::Host::Ipv4(ip)) if !is_public_ip(IpAddr::V4(ip)) => {
            Err(format!("refusing to fetch non-public address {ip}"))
        }
        Some(url::Host::Ipv6(ip)) if !is_public_ip(IpAddr::V6(ip)) => {
            Err(format!("refusing to fetch non-public address {ip}"))
        }
        Some(url::Host::Domain(host))
            if host.eq_ignore_ascii_case("localhost")
                || host.to_ascii_lowercase().ends_with(".localhost") =>
        {
            Err(format!("refusing to fetch local host {host}"))
        }
        Some(_) => Ok(()),
        None => Err("URL has no host".to_owned()),
    }
}

#[derive(Debug, Clone, Copy)]
struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
            let public: Vec<_> = addrs.filter(|addr| is_public_ip(addr.ip())).collect();
            if public.is_empty() {
                return Err(format!("host `{host}` resolves only to non-public addresses").into());
            }
            let resolved: Addrs = Box::new(public.into_iter());
            Ok(resolved)
        })
    }
}

/// Client for fetching user-supplied URLs and their sub-resources.
pub(crate) fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .dns_resolver(Arc::new(PublicOnlyResolver))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            match validate_public_url(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(e) => attempt.error(e),
            }
        }))
}

#[derive(Debug)]
pub(crate) struct FetchedResource {
    /// URL after redirects (what the bytes actually came from).
    pub final_url: Url,
    pub status: u16,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Fetch one resource. Non-2xx responses are errors and are not stored.
pub(crate) async fn fetch(
    client: &reqwest::Client,
    url: Url,
    max_bytes: usize,
) -> Result<FetchedResource, FetchError> {
    validate_public_url(&url).map_err(FetchError::Blocked)?;
    let response = client.get(url.clone()).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::Status(status.as_u16()));
    }

    if response
        .content_length()
        .is_some_and(|len| len > u64::try_from(max_bytes).unwrap_or(u64::MAX))
    {
        return Err(FetchError::TooLarge(max_bytes));
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned();

    let final_url = response.url().clone();

    // Read incrementally so a server that lies about Content-Length (or streams
    // forever) cannot exhaust memory.
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(FetchError::TooLarge(max_bytes));
        }
        bytes.extend_from_slice(&chunk);
    }

    if bytes.is_empty() {
        return Err(FetchError::Empty);
    }

    Ok(FetchedResource {
        final_url,
        status: status.as_u16(),
        content_type,
        bytes,
    })
}
