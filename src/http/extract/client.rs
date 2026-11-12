use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts};
use http::{header::USER_AGENT, request::Parts};

use crate::{auth::ClientInfo, error::AppError};

impl<S: Send + Sync> FromRequestParts<S> for ClientInfo {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let ConnectInfo(addr) = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .copied()
            .ok_or_else(|| AppError::Internal("missing ConnectInfo".into()))?;

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

        let ua = parts.headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
        Ok(Self::new(addr.ip(), ua))
    }
}
