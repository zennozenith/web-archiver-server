use serde::Serialize;

use std::{borrow::Borrow, collections::HashSet, fmt, ops::Deref, str::FromStr};

use crate::error::AppError;

pub(crate) const MAX_TAG_LEN: usize = 64;
pub(crate) const MAX_TAGS: usize = 50;

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, sqlx::Type, Serialize)]
#[sqlx(transparent)]
pub struct Name(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidName {
    #[error("tag name cannot be empty")]
    Empty,
    #[error("tag name exceeds {max} characters (got {len})")]
    TooLong { len: usize, max: usize },
}

impl Name {
    /// Sanitize `raw` (trim, lowercase, collapse non-alphanumerics to `-`)
    /// and validate the result.
    pub fn new(raw: &str) -> Result<Self, InvalidName> {
        Self::try_from(raw)
    }

    /// Sanitize without validating length/emptiness. Callers who only need
    /// a normalized string (e.g. to feed a search index) can use this.
    pub fn sanitize(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut prev_hyphen = true; // trim leading hyphens

        for c in input.chars() {
            if c.is_alphanumeric() {
                out.extend(c.to_lowercase());
                prev_hyphen = false;
            } else if !prev_hyphen {
                out.push('-');
                prev_hyphen = true;
            }
        }

        if out.ends_with('-') {
            out.pop();
        }
        out
    }

    /// Borrow the sanitized name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume into the underlying `String`.
    pub fn into_inner(self) -> String {
        self.0
    }
}

pub(crate) fn clean_name(raw: &str) -> Result<Name, AppError> {
    Name::new(raw).map_err(|e| AppError::invalid("invalid_tag", e.to_string()))
}

/// Trim, drop empties, de-duplicate case-insensitively, enforce limits.
pub(crate) fn clean_names(raw: &[String]) -> Result<Vec<Name>, AppError> {
    if raw.len() > MAX_TAGS {
        return Err(AppError::invalid(
            "too_many_tags",
            format!("at most {MAX_TAGS} tags allowed"),
        ));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for r in raw {
        if r.trim().is_empty() {
            continue;
        }
        let name = clean_name(r)?;
        if seen.insert(name.to_lowercase()) {
            out.push(name);
        }
    }
    Ok(out)
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Name {
    type Err = InvalidName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

impl TryFrom<&str> for Name {
    type Error = InvalidName;

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        let s = Self::sanitize(raw);
        if s.is_empty() {
            return Err(InvalidName::Empty);
        }
        let len = s.chars().count();
        if len > MAX_TAG_LEN {
            return Err(InvalidName::TooLong {
                len,
                max: MAX_TAG_LEN,
            });
        }
        Ok(Self(s))
    }
}

impl TryFrom<String> for Name {
    type Error = InvalidName;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::try_from(raw.as_str())
    }
}

impl Deref for Name {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<Name> for String {
    fn from(value: Name) -> Self {
        value.0
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Name {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

/// Colors are palette keys like "red" or "deep-blue", never hex values.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, sqlx::Type, Serialize)]
#[sqlx(transparent)]
pub(crate) struct Color(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("color must be a palette key: lowercase letters, digits and '-' (max 32), got {0}")]
pub(crate) struct InvalidColor(String);

impl Color {
    /// Max length of a palette key.
    pub(crate) const MAX_LEN: usize = 32;

    /// Construct a `Color` from a palette key, validating the format.
    pub(crate) fn new(s: impl Into<String>) -> Result<Self, InvalidColor> {
        let s = s.into();
        let ok = !s.is_empty()
            && s.len() <= Self::MAX_LEN
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if ok {
            Ok(Self(s))
        } else {
            Err(InvalidColor(s))
        }
    }

    // /// Borrow the underlying palette key.
    // pub(crate) fn as_str(&self) -> &str {
    //     &self.0
    // }

    // /// Consume into the underlying `String`.
    // pub(crate) fn into_inner(self) -> String {
    //     self.0
    // }
}

/// Colors are palette keys like "red" or "deep-blue", never hex values.
pub(crate) fn validate_color(color: Option<&str>) -> Result<Option<Color>, AppError> {
    color.map(str::trim).filter(|c| !c.is_empty()).map_or_else(
        || Ok(None),
        |c| {
            Color::new(c)
                .map(Some)
                .map_err(|e| AppError::invalid("invalid_color", e.to_string()))
        },
    )
}

impl AsRef<str> for Color {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Color {
    type Err = InvalidColor;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for Color {
    type Error = InvalidColor;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl TryFrom<&str> for Color {
    type Error = InvalidColor;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

pub(crate) struct Merge {
    from: i64,
    into: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot merge a tag into itself")]
pub(crate) struct InvalidMerge;

impl Merge {
    pub(crate) const fn new(from: i64, into: i64) -> Result<Self, InvalidMerge> {
        if from == into {
            return Err(InvalidMerge);
        }
        Ok(Self { from, into })
    }

    pub(crate) const fn from(&self) -> i64 {
        self.from
    }

    pub(crate) const fn into_(&self) -> i64 {
        self.into
    }
}
