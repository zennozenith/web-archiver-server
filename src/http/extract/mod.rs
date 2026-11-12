#![expect(
    dead_code,
    reason = "alternative extractors and deserializers are kept for future routes"
)]

pub(crate) mod auth_user;
pub(crate) mod body;
pub(crate) mod client;
pub(crate) mod pagination;
