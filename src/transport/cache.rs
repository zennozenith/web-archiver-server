//! `Cache-Control` policies shared by the content-serving endpoints.

/// User-owned content that may change (thumbnails, captured pages).
pub(crate) const PRIVATE_DAY: &str = "private, max-age=86400";
/// Content-addressed bytes: safe to cache forever, scoped to the owner.
pub(crate) const PRIVATE_IMMUTABLE: &str = "private, max-age=31536000, immutable";
/// Built frontend assets, identical for every visitor.
pub(crate) const PUBLIC_IMMUTABLE: &str = "public, max-age=31536000, immutable";
