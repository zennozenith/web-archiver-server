//! Grouping helper shared by the list queries.

use std::collections::HashMap;
use std::hash::Hash;

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
