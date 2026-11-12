//! Shared plumbing for the HTML routes: the session extractor, error
//! responses, the template renderer and the multi-fragment response builder.

mod newer;
mod request;
mod response;

pub(crate) use request::*;
pub(crate) use response::*;
