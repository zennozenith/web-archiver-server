use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// A normalized `type/subtype` media type (lowercase, no parameters).
/// Defaults to `application/octet-stream`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub(crate) struct MimeType(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid MIME type: {0:?}")]
pub(crate) struct InvalidMimeType(String);

impl MimeType {
    pub(crate) const OCTET_STREAM: &'static str = "application/octet-stream";

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    #[expect(dead_code, reason = "Might be needed later")]
    pub(crate) fn is_default(&self) -> bool {
        self.0 == Self::OCTET_STREAM
    }

    #[expect(dead_code, reason = "Might be needed later")]
    /// The part before `/`, e.g. `image`.
    pub(crate) fn type_(&self) -> &str {
        self.0.split_once('/').map_or("", |(t, _)| t)
    }

    #[expect(dead_code, reason = "Might be needed later")]
    /// The part after `/`, e.g. `png`.
    pub(crate) fn subtype(&self) -> &str {
        self.0.split_once('/').map_or("", |(_, s)| s)
    }

    #[expect(dead_code, reason = "Might be needed later")]
    /// Use with `#[serde(default, deserialize_with = "MimeType::deserialize_lenient")]`.
    /// Accepts `null` and invalid strings, falling back to the default.
    pub(crate) fn deserialize_lenient<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Option::<String>::deserialize(deserializer)?.into())
    }
}

impl Default for MimeType {
    fn default() -> Self {
        Self(Self::OCTET_STREAM.to_owned())
    }
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
}

impl FromStr for MimeType {
    type Err = InvalidMimeType;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Drop parameters like `; charset=utf-8`, then normalize.
        let essence = s
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        match essence.split_once('/') {
            Some((t, sub)) if is_token(t) && is_token(sub) => Ok(Self(essence)),
            _ => Err(InvalidMimeType(s.to_owned())),
        }
    }
}

/// Lenient conversion used when reading from the DB:
/// `NULL` or garbage falls back to the default.
impl From<Option<String>> for MimeType {
    fn from(v: Option<String>) -> Self {
        v.and_then(|s| s.parse().ok()).unwrap_or_default()
    }
}

impl fmt::Display for MimeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for MimeType {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Serialize for MimeType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for MimeType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(de::Error::custom)
    }
}
