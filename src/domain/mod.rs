//! Validated domain value types and typed row ids.
//!
//! These types carry invariants (a normalised tag name, a non-zero limit, a
//! typed id) and are shared by persistence, services and transports. They do
//! no I/O and must not depend on HTTP or SQL execution.

pub(crate) mod ids;
pub(crate) mod tag;

pub(crate) use tag::*;
