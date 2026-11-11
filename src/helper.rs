use crate::error::AppError;

use std::{fmt, ops::Deref, str::FromStr};

/// Escapes `%`, `_` and `\` so user text is matched literally by `LIKE ... ESCAPE '\'`.
pub(crate) fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

// ───────────────────────────────────────────────────────────────────────────
//
// ───────────────────────────────────────────────────────────────────────────

/// An error returned when parsing a boolean string fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid boolean value '{invalid_value}': expected true/false, yes/no, 1/0, or on/off")]
pub(crate) struct ParseBoolError {
    invalid_value: String,
}

impl From<ParseBoolError> for AppError {
    fn from(err: ParseBoolError) -> Self {
        Self::invalid("invalid_bool_value", err.to_string())
    }
}

/// Parses a string slice into a boolean value, supporting common truthy and falsy representations.
///
/// The input is trimmed and converted to lowercase ASCII before matching:
/// - **Truthy:** `"1"`, `"true"`, `"yes"`, `"on"`
/// - **Falsy:** `"0"`, `"false"`, `"no"`, `"off"`
///
/// # Errors
///
/// Returns a [`ParseBoolError`] containing the offending input if the string
/// does not match any recognized boolean representation.
///
/// # Examples
///
/// ```rust
/// // assert_eq!(parse_bool("true").unwrap(), true);
/// // assert_eq!(parse_bool("OFF").unwrap(), false);
/// // assert!(parse_bool("maybe").is_err());
/// ```
pub(crate) fn parse_bool(s: &str) -> Result<bool, ParseBoolError> {
    let trimmed = s.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ParseBoolError {
            invalid_value: trimmed.to_owned(),
        }),
    }
}

// ───────────────────────────────────────────────────────────────────────────
//
// ───────────────────────────────────────────────────────────────────────────

/// An error returned when attempting to create a `NonEmptyString` from an empty string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("string cannot be empty")]
pub(crate) struct EmptyStringError;

impl From<EmptyStringError> for AppError {
    fn from(err: EmptyStringError) -> Self {
        Self::invalid("empty_string", err.to_string())
    }
}

/// A validated string wrapper that guarantees the inner string is never empty.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, sqlx::Type)]
#[sqlx(transparent)]
#[repr(transparent)]
pub(crate) struct NonEmptyString(Box<str>);

impl NonEmptyString {
    /// Attempts to create a `NonEmptyString` from any string-like type.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyStringError`] if the input string is empty.
    pub(crate) fn new(s: impl Into<String>) -> Result<Self, EmptyStringError> {
        let s = s.into();
        if s.is_empty() {
            Err(EmptyStringError)
        } else {
            Ok(Self(s.into_boxed_str()))
        }
    }

    /// Creates a `NonEmptyString` from a trimmed string, rejecting strings
    /// that are empty or consist solely of whitespace.
    pub(crate) fn trimmed(s: impl AsRef<str>) -> Result<Self, EmptyStringError> {
        let trimmed = s.as_ref().trim();
        if trimmed.is_empty() {
            Err(EmptyStringError)
        } else {
            Ok(Self(trimmed.into()))
        }
    }

    /// Returns a string slice of the inner data.
    #[expect(dead_code, reason = "Might be needed later")]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the owned `String`.
    #[expect(dead_code, reason = "Might be needed later")]
    pub(crate) fn into_inner(self) -> String {
        self.0.into_string()
    }

    /// Trims whitespace from a string slice and returns an owned, non-empty `String`.
    ///
    /// Returns `None` if the input string is empty or consists entirely of whitespace.
    /// Otherwise, returns `Some` containing the trimmed string.
    ///
    /// # Examples
    ///
    /// ```rust
    /// assert_eq!(non_empty("  hello world  "), Some("hello world".to_string()));
    /// assert_eq!(non_empty("   "), None);
    /// assert_eq!(non_empty(""), None);
    /// ```
    pub(crate) fn non_empty(s: &str) -> Option<String> {
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_owned())
    }
}

// --- Ergonomic Conversions & Traits ---

impl TryFrom<String> for NonEmptyString {
    type Error = EmptyStringError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl FromStr for NonEmptyString {
    type Err = EmptyStringError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

/// Allows `&NonEmptyString` to be seamlessly passed into functions expecting `&str`.
impl Deref for NonEmptyString {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for NonEmptyString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NonEmptyString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// use serde::{Deserialize, Deserializer, Serialize, Serializer};

// impl Serialize for NonEmptyString {
//     fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
//     where
//         S: Serializer,
//     {
//         // A NonEmptyString serializes just like a regular String
//         self.0.serialize(serializer)
//     }
// }

// impl<'de> Deserialize<'de> for NonEmptyString {
//     fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
//     where
//         D: Deserializer<'de>,
//     {
//         // First deserialize into a standard String
//         let s = String::deserialize(deserializer)?;

//         // Then validate it through NonEmptyString::new, mapping the error
//         // into a Serde custom deserialization error
//         Self::new(s).map_err(serde::de::Error::custom)
//     }
// }
