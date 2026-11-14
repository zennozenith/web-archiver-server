//! Three-state fields for PATCH payloads.

use serde::{Deserialize, Deserializer};

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
