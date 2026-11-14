//! htmx 4 support for the HTML routes.
//!
//! # Overview
//!
//! | Concern                     | Items                                         |
//! |-----------------------------|-----------------------------------------------|
//! | Reading htmx request headers| [`HtmxRequest`], [`RequestType`]              |
//! | Single-response variants    | [`HtmxResponse`], [`PartialSwap`], [`HxHeaders`] |
//! | Multi-template responses    | [`Fragments`], [`ToastKind`]                  |
//! | Plain template rendering    | [`Render`]                                    |
//! | Error responses             | [`PageError`] (full page), [`ActionError`] (toast) |
//!
//! # Conventions
//!
//! * Routes that serve both full pages and fragments extract [`HtmxRequest`]
//!   and call [`HtmxRequest::render_tpl`]; the response then carries a `Vary`
//!   header so caches key on the htmx request headers.
//! * Full-page routes return `Result<_>`; htmx action routes
//!   return `Result<_>`.
//! * Server errors (5xx) are logged and shown to the user with a generic
//!   message; internal details never reach the client.

#![expect(
    dead_code,
    reason = "the htmx helpers cover the full protocol; not every variant is used yet"
)]

mod error;
mod fragments;
mod header;
mod render;
mod request;
mod response;

pub(crate) use error::*;
pub(crate) use fragments::*;
pub(crate) use render::*;
pub(crate) use request::*;
pub(crate) use response::*;
