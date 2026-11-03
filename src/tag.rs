//! Validated tag value types: names, palette colors, and merge requests.

use std::{borrow::Borrow, collections::HashSet, fmt, str::FromStr};

use serde::Serialize;

use crate::{db::helper::id_type, error::AppError};

pub(crate) const MAX_NAME_LEN: usize = 64;
pub(crate) const MAX_TAGS_PER_REQUEST: usize = 50;

// ───────────────────────────────────────────────────────────────────────────
// TagId
// ───────────────────────────────────────────────────────────────────────────

id_type! {
    /// Primary key of a tag. A distinct type so it cannot be mixed up with other
    /// row ids (page ids, pending ids) or with plain integers such as counts.
    pub(crate) struct TagId;
}

// ───────────────────────────────────────────────────────────────────────────
// TagName
// ───────────────────────────────────────────────────────────────────────────

/// A normalized tag name: lowercase alphanumerics separated by single `-`.
///
/// Construct via [`FromStr`] / [`TryFrom<&str>`]; the invariant is enforced there.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, sqlx::Type, Serialize)]
#[sqlx(transparent)]
pub struct TagName(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidTagName {
    #[error("tag name cannot be empty")]
    Empty,
    #[error("tag name exceeds {max} characters (got {len})")]
    TooLong { len: usize, max: usize },
}

impl TagName {
    /// Lowercase `input` and collapse every run of non-alphanumerics into a
    /// single `-`, with no leading or trailing separator.
    ///
    /// Does not validate length or emptiness, so it is also usable for
    /// normalizing search queries.
    pub fn normalize(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut pending_separator = false;

        for c in input.chars() {
            if !c.is_alphanumeric() {
                pending_separator = true;
                continue;
            }
            if pending_separator && !out.is_empty() {
                out.push('-');
            }
            pending_separator = false;
            out.extend(c.to_lowercase());
        }
        out
    }

    // pub fn normalize_slower(input: &str) -> String {
    //     input
    //         .split(|c: char| !c.is_alphanumeric())
    //         .filter(|word| !word.is_empty())
    //         .map(str::to_lowercase)
    //         .collect::<Vec<_>>()
    //         .join("-")
    // }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TagName {
    type Err = InvalidTagName;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let name = Self::normalize(raw);
        if name.is_empty() {
            return Err(InvalidTagName::Empty);
        }
        let len = name.chars().count();
        if len > MAX_NAME_LEN {
            return Err(InvalidTagName::TooLong {
                len,
                max: MAX_NAME_LEN,
            });
        }
        Ok(Self(name))
    }
}

impl TryFrom<&str> for TagName {
    type Error = InvalidTagName;

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl AsRef<str> for TagName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for TagName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TagName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<TagName> for String {
    fn from(name: TagName) -> Self {
        name.0
    }
}

impl From<InvalidTagName> for AppError {
    fn from(err: InvalidTagName) -> Self {
        Self::invalid("invalid_tag", err.to_string())
    }
}

/// Parse a batch of raw names: skips blank entries, de-duplicates by
/// normalized form (first occurrence wins), and enforces the batch limit.
pub(crate) fn parse_names(raw: &[String]) -> Result<Vec<TagName>, AppError> {
    if raw.len() > MAX_TAGS_PER_REQUEST {
        return Err(AppError::invalid(
            "too_many_tags",
            format!("at most {MAX_TAGS_PER_REQUEST} tags allowed"),
        ));
    }

    let mut seen = HashSet::new();
    let mut names = Vec::with_capacity(raw.len());
    for entry in raw.iter().filter(|s| !s.trim().is_empty()) {
        let name: TagName = entry.parse()?;
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }
    Ok(names)
}

pub(crate) fn comma_separated_tags(raw: Option<&String>) -> Result<Vec<TagName>, AppError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let names: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    parse_names(&names)
}

// ───────────────────────────────────────────────────────────────────────────
// TagColor
// ───────────────────────────────────────────────────────────────────────────

/// A palette key such as `"red"` or `"deep-blue"` — never a hex value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, sqlx::Type, Serialize)]
#[sqlx(transparent)]
pub(crate) struct TagColor(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("color must be a palette key: lowercase letters, digits and '-' (max {max}), got {0:?}", max = TagColor::MAX_LEN)]
pub(crate) struct InvalidTagColor(String);

impl TagColor {
    pub(crate) const MAX_LEN: usize = 32;
}

impl FromStr for TagColor {
    type Err = InvalidTagColor;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let is_valid = !s.is_empty()
            && s.len() <= Self::MAX_LEN
            && s.bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'));

        if is_valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(InvalidTagColor(s.to_owned()))
        }
    }
}

impl TryFrom<&str> for TagColor {
    type Error = InvalidTagColor;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl AsRef<str> for TagColor {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TagColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<InvalidTagColor> for AppError {
    fn from(err: InvalidTagColor) -> Self {
        Self::invalid("invalid_color", err.to_string())
    }
}

/// Parse an optional color; `None` and blank strings both mean "no color".
pub(crate) fn parse_optional_color(raw: Option<&str>) -> Result<Option<TagColor>, InvalidTagColor> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::parse)
        .transpose()
}

// ───────────────────────────────────────────────────────────────────────────
// TagMerge
// ───────────────────────────────────────────────────────────────────────────

/// A request to fold `source` into `target`. The two ids are guaranteed distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TagMerge {
    source: TagId,
    target: TagId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cannot merge a tag into itself")]
pub(crate) struct SelfMerge;

impl TagMerge {
    pub(crate) const fn new(source: TagId, target: TagId) -> Result<Self, SelfMerge> {
        if source.0 == target.0 {
            return Err(SelfMerge);
        }
        Ok(Self { source, target })
    }

    /// The tag that is absorbed and deleted.
    pub(crate) const fn source(self) -> TagId {
        self.source
    }

    /// The tag that survives and receives the source's associations.
    pub(crate) const fn target(self) -> TagId {
        self.target
    }
}
