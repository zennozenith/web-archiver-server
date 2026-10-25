use crate::error::{AppError, Result};

use std::borrow::Borrow;
use std::collections::HashSet;
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

pub const MAX_TAG_LEN: usize = 64;
pub const MAX_TAGS: usize = 50;

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct CleanTag(String);

impl CleanTag {
    /// Creates a new `CleanTag`, sanitizing the input.
    pub fn new(input: impl Into<String>) -> Self {
        Self(Self::sanitize(&input.into()))
    }

    /// Returns the inner string as a `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the `CleanTag`, returning the inner `String`.
    pub fn into_inner(self) -> String {
        self.0
    }

    /// Sanitizes a tag by trimming, lowercasing, and replacing
    /// any run of non-alphanumeric characters with a single hyphen.
    fn sanitize(input: &str) -> String {
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

        // trim trailing hyphen
        if out.ends_with('-') {
            out.pop();
        }
        out
    }
}

impl Deref for CleanTag {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for CleanTag {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for CleanTag {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<String> for CleanTag {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for CleanTag {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<CleanTag> for String {
    fn from(value: CleanTag) -> Self {
        value.0
    }
}

impl FromStr for CleanTag {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

impl fmt::Display for CleanTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for CleanTag {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for CleanTag {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for CleanTag {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

pub fn clean_tag_name(raw: &str) -> Result<CleanTag> {
    let name = CleanTag::new(raw);
    if name.is_empty() {
        return Err(AppError::invalid("invalid_tag", "tag name cannot be empty"));
    }
    if name.chars().count() > MAX_TAG_LEN {
        return Err(AppError::invalid(
            "invalid_tag",
            format!("tag name exceeds {MAX_TAG_LEN} characters"),
        ));
    }
    Ok(name)
}

/// Trim, drop empties, de-duplicate case-insensitively, enforce limits.
pub fn clean_tags(raw: &[String]) -> Result<Vec<CleanTag>> {
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
        let name = clean_tag_name(r)?;
        if seen.insert(name.to_lowercase()) {
            out.push(name);
        }
    }
    Ok(out)
}
