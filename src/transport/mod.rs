pub(crate) mod asset;
pub(crate) mod cache;
pub(crate) mod cookie;
pub(crate) mod multipart;
pub(crate) mod security;
pub(crate) mod serve;

pub(crate) mod api;
pub(crate) mod dto;
pub(crate) mod extract;
pub(crate) mod htmx;
pub(crate) mod web;

/// Where unauthenticated users are sent.
pub(crate) const LOGIN_PATH: &str = "/login";
