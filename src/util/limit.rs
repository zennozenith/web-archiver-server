//! A validated, pagination-style `Limit`.
//!
//! Invariant: `1 <= limit <= Limit::MAX`. Invalid values can't be constructed,
//! so downstream code (SQL `LIMIT`, page sizes, ...) never has to re-check.
//! ```

use std::{fmt, num::NonZeroU32, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

const MAX_RAW: u32 = 1_000;
const DEFAULT_RAW: u32 = 50;

/// A non-zero, upper-bounded result limit.
///
/// `Option<Limit>` is the same size as `Limit` thanks to the `NonZeroU32` niche.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Limit(NonZeroU32);

/// Error returned when a value can't be turned into a [`Limit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitError {
    /// Value was outside `1..=Limit::MAX` (or didn't fit in a `u32`).
    OutOfRange,
    /// String input wasn't a valid unsigned integer.
    Parse(std::num::ParseIntError),
}

impl Limit {
    /// Smallest allowed limit (1).
    pub const MIN: Self = Self(NonZeroU32::MIN);
    /// Largest allowed limit.
    pub const MAX: Self = Self::from_const(MAX_RAW);
    /// Default limit used by `Limit::default()`.
    pub const DEFAULT: Self = Self::from_const(DEFAULT_RAW);

    /// Compile-time constructor; panics (at compile time in const context) if invalid.
    ///
    /// ```ignore
    /// const PAGE: Limit = Limit::from_const(25);
    /// ```
    pub const fn from_const(n: u32) -> Self {
        assert!(n >= 1 && n <= MAX_RAW, "limit out of range");
        match NonZeroU32::new(n) {
            Some(nz) => Self(nz),
            #[expect(
                clippy::unreachable,
                reason = "Only called from const context. Should never be reachable because of above assert"
            )]
            None => unreachable!(),
        }
    }

    /// Fallible constructor.
    pub const fn new(n: u32) -> Result<Self, LimitError> {
        if n < 1 || n > MAX_RAW {
            return Err(LimitError::OutOfRange);
        }
        match NonZeroU32::new(n) {
            Some(nz) => Ok(Self(nz)),
            None => Err(LimitError::OutOfRange),
        }
    }

    /// Infallible constructor that clamps into `1..=MAX`.
    pub const fn clamped(n: u32) -> Self {
        if n < 1 {
            Self::MIN
        } else if n > MAX_RAW {
            Self::MAX
        } else {
            Self::from_const(n)
        }
    }

    /// The raw value.
    pub const fn get(self) -> u32 {
        self.0.get()
    }

    /// The raw value as `usize` (handy for `Vec::with_capacity`, `.take(n)`).
    pub fn as_usize(self) -> usize {
        usize::try_from(self.0.get()).unwrap_or(usize::MAX)
    }

    /// Convenience for optional query params: `None` -> default.
    pub fn or_default(opt: Option<Self>) -> Self {
        opt.unwrap_or_default()
    }

    #[expect(
        clippy::as_conversions,
        reason = "using a potentially dangerous silent `as` conversion"
    )]
    /// Number of pages needed to cover `total` items.
    pub const fn pages(self, total: u64) -> u64 {
        total.div_ceil(self.0.get() as u64)
    }
}

impl Default for Limit {
    fn default() -> Self {
        Self::DEFAULT
    }
}

// ---------- Display / errors ----------

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.get(), f)
    }
}

impl fmt::Display for LimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange => write!(f, "limit must be between 1 and {MAX_RAW}"),
            Self::Parse(e) => write!(f, "invalid limit: {e}"),
        }
    }
}

impl std::error::Error for LimitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(e) => Some(e),
            Self::OutOfRange => None,
        }
    }
}

impl From<std::num::ParseIntError> for LimitError {
    fn from(e: std::num::ParseIntError) -> Self {
        Self::Parse(e)
    }
}

// ---------- Conversions ----------

impl FromStr for Limit {
    type Err = LimitError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.trim().parse::<u32>()?)
    }
}

macro_rules! impl_try_from_int {
    ($($t:ty),* $(,)?) => {$(
        impl TryFrom<$t> for Limit {
            type Error = LimitError;
            fn try_from(v: $t) -> Result<Self, Self::Error> {
                u32::try_from(v)
                    .map_err(|_| LimitError::OutOfRange)
                    .and_then(Self::new)
            }
        }
    )*};
}
impl_try_from_int!(u32, u64, usize, i32, i64);

macro_rules! impl_from_limit {
    ($($t:ty),* $(,)?) => {$(
        impl From<Limit> for $t {
            fn from(l: Limit) -> Self { l.get() as $t }
        }
    )*};
}
impl_from_limit!(u32, u64, usize, i64);

impl From<Limit> for NonZeroU32 {
    fn from(l: Limit) -> Self {
        l.0
    }
}

impl PartialEq<u32> for Limit {
    fn eq(&self, other: &u32) -> bool {
        self.get() == *other
    }
}

impl PartialEq<Limit> for u32 {
    fn eq(&self, other: &Limit) -> bool {
        *self == other.get()
    }
}

// ---------- Serde ----------

impl Serialize for Limit {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(self.get())
    }
}

impl<'de> Deserialize<'de> for Limit {
    /// Validates on the way in, so bad input is rejected at the boundary.
    /// Works with JSON numbers and with `serde_urlencoded` query strings.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let n = u32::deserialize(d)?;
        Self::new(n).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    struct Query {
        #[serde(default)]
        limit: Limit,
        q: String,
    }

    #[test]
    fn bounds() {
        assert!(Limit::new(0).is_err());
        assert!(Limit::new(1).is_ok());
        assert!(Limit::new(MAX_RAW).is_ok());
        assert!(Limit::new(MAX_RAW + 1).is_err());
        assert_eq!(Limit::clamped(0), Limit::MIN);
        assert_eq!(Limit::clamped(u32::MAX), Limit::MAX);
    }

    #[test]
    fn conversions() {
        assert_eq!(Limit::try_from(10usize).unwrap(), 10u32);
        assert!(Limit::try_from(-1i64).is_err());
        assert_eq!("25".parse::<Limit>().unwrap().get(), 25);
        assert!(matches!("abc".parse::<Limit>(), Err(LimitError::Parse(_))));
        assert_eq!(usize::from(Limit::DEFAULT), 50);
        assert_eq!(std::mem::size_of::<Option<Limit>>(), 4);
    }

    #[test]
    fn pages() {
        assert_eq!(Limit::from_const(10).pages(95), 10);
        assert_eq!(Limit::from_const(10).pages(0), 0);
    }

    #[test]
    fn serde_roundtrip() {
        let q = Query {
            limit: Limit::from_const(20),
            q: "x".into(),
        };
        let json = serde_json::to_string(&q).unwrap();
        assert_eq!(json, r#"{"limit":20,"q":"x"}"#);
        assert_eq!(serde_json::from_str::<Query>(&json).unwrap(), q);
    }

    #[test]
    fn serde_default_and_validation() {
        let q: Query = serde_json::from_str(r#"{"q":"x"}"#).unwrap();
        assert_eq!(q.limit, Limit::DEFAULT);

        assert!(serde_json::from_str::<Limit>("0").is_err());
        assert!(serde_json::from_str::<Limit>("1001").is_err());
        assert!(serde_json::from_str::<Limit>("-5").is_err());
        let err = serde_json::from_str::<Limit>("0").unwrap_err().to_string();
        assert!(err.contains("between 1 and 1000"));
    }
}
