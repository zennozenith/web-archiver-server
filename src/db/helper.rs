use std::{collections::HashMap, fmt, hash::Hash, ops::Deref, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize};

// ───────────────────────────────────────────────────────────────────────────
// Typed ids
// ───────────────────────────────────────────────────────────────────────────

/// Declares a typed row id: a transparent `i64` newtype with the usual derives.
///
/// ```ignore
/// id_type! {
///     /// Primary key of a page.
///     pub(crate) struct PageId;
/// }
/// ```
macro_rules! id_type {
    ($(#[$meta:meta])* $vis:vis struct $name:ident;) => {
        $(#[$meta])*
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
            ::sqlx::Type, ::serde::Serialize, ::serde::Deserialize,
        )]
        #[sqlx(transparent)]
        #[serde(transparent)]
        $vis struct $name(i64);

        impl $name {

            #[expect(clippy::allow_attributes, reason = "Might be needed later")]
            #[allow(dead_code, reason = "Might be needed later")]
            pub(crate) const fn new(id: i64) -> Self {
                Self(id)
            }

            #[expect(clippy::allow_attributes, reason = "Might be needed later")]
            #[allow(dead_code, reason = "Might be needed later")]
            pub(crate) const fn get(self) -> i64 {
                self.0
            }
        }

        impl From<i64> for $name {
            fn from(id: i64) -> Self {
                Self(id)
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}
pub(crate) use id_type;

use crate::error::AppError;

// ───────────────────────────────────────────────────────────────────────────
// Create-if-absent
// ───────────────────────────────────────────────────────────────────────────

/// Result of a create-if-absent operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateOutcome<T> {
    /// A new row was inserted.
    Created(T),
    /// A row with the same unique key already existed; it is returned untouched.
    Existing(T),
}

impl<T> CreateOutcome<T> {
    pub(crate) const fn is_created(&self) -> bool {
        matches!(self, Self::Created(_))
    }

    pub(crate) fn into_inner(self) -> T {
        match self {
            Self::Created(value) | Self::Existing(value) => value,
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// PATCH fields
// ───────────────────────────────────────────────────────────────────────────

/// Three-state field for PATCH payloads.
///
/// Use with `#[serde(default)]` so a missing key maps to `Unchanged`,
/// an explicit `null` to `Clear`, and a value to `Set`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum FieldPatch<T> {
    #[default]
    Unchanged,
    Clear,
    Set(T),
}

impl<T: Deref> FieldPatch<T> {
    /// Borrow the contained value as its deref target (`FieldPatch<String>` -> `FieldPatch<&str>`).
    #[expect(dead_code, reason = "Might be needed later")]
    pub(crate) fn as_deref(&self) -> FieldPatch<&T::Target> {
        match self {
            Self::Unchanged => FieldPatch::Unchanged,
            Self::Clear => FieldPatch::Clear,
            Self::Set(value) => FieldPatch::Set(&**value),
        }
    }
}

impl<T> FieldPatch<T> {
    /// Apply the patch to the field's current value.
    pub(crate) fn apply(self, current: Option<T>) -> Option<T> {
        match self {
            Self::Unchanged => current,
            Self::Clear => None,
            Self::Set(value) => Some(value),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for FieldPatch<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Option::<T>::deserialize(deserializer)?.map_or(Self::Clear, Self::Set))
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Keyset pagination
// ───────────────────────────────────────────────────────────────────────────

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
#[serde(rename_all = "camelCase")]
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

// ───────────────────────────────────────────────────────────────────────────
// Grouping
// ───────────────────────────────────────────────────────────────────────────

/// Group `(key, value)` pairs into a map, preserving the order of values per key.
pub(crate) fn group_by<K: Hash + Eq, V>(
    pairs: impl IntoIterator<Item = (K, V)>,
) -> HashMap<K, Vec<V>> {
    let mut groups: HashMap<K, Vec<V>> = HashMap::new();
    for (key, value) in pairs {
        groups.entry(key).or_default().push(value);
    }
    groups
}

// // ───────────────────────────────────────────────────────────────────────────
// // Matching
// // ───────────────────────────────────────────────────────────────────────────
// #[derive(Debug, Default, Deserialize)]
// #[serde(rename_all = "lowercase")]
// pub(crate) enum MatchMode {
//     #[default]
//     All,
//     Any,
// }
