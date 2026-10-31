#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Inserted<T, U = i64> {
    New(T),
    Existing(U),
}
