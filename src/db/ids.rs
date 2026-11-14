//! Typed row ids.

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
        #[serde(transparent, rename_all = "camelCase")]
        $vis struct $name(i64);

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
