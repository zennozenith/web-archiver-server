//! Keyset pagination.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid cursor")]
pub(crate) struct InvalidCursor;

impl From<InvalidCursor> for AppError {
    fn from(e: InvalidCursor) -> Self {
        Self::BadRequest(e.to_string())
    }
}

/// Position in a keyset-paginated listing keyed by a single integer
/// (an id, or a computed version number).
///
/// Travels over the wire as a decimal string (`"42"`), so clients treat it as
/// opaque, and malformed input is rejected at deserialization time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub(crate) struct Cursor(i64);

impl Cursor {
    pub(crate) const fn new(position: i64) -> Self {
        Self(position)
    }

    pub(crate) const fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for Cursor {
    type Err = InvalidCursor;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self).map_err(|_| InvalidCursor)
    }
}

impl From<Cursor> for String {
    fn from(cursor: Cursor) -> Self {
        cursor.to_string()
    }
}

impl TryFrom<String> for Cursor {
    type Error = InvalidCursor;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// One page of a keyset-paginated listing.
///
/// Named `CursorPage` (not `Page`) because "page" already means an archived
/// web page in this codebase.
#[derive(Debug, Serialize)]
pub(crate) struct CursorPage<T, C = Cursor> {
    pub items: Vec<T>,
    /// Send back as `cursor` to fetch the next page. `None` on the last page.
    pub next_cursor: Option<C>,
}

impl<T, C> CursorPage<T, C> {
    /// Build a page from rows fetched with `LIMIT limit + 1`.
    ///
    /// The extra row only signals that more data exists: it is dropped, and the
    /// cursor is taken from the last row that is kept.
    pub(crate) fn from_overfetched(
        mut rows: Vec<T>,
        limit: usize,
        cursor_of: impl FnOnce(&T) -> C,
    ) -> Self {
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = rows.last().filter(|_| has_more).map(cursor_of);
        Self {
            items: rows,
            next_cursor,
        }
    }
}
