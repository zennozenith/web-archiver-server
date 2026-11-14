pub(crate) mod admin;
pub(crate) mod archive;
pub(crate) mod auth;
pub(crate) mod user;

use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct OkDto {
    ok: bool,
}

impl OkDto {
    pub(crate) const fn new() -> Self {
        Self { ok: true }
    }
}
