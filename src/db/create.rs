//! Create-if-absent results.

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
