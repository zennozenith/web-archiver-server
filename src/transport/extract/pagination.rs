use serde::Deserialize;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

// NOTE:
// One caveat: serde(flatten) with Query can’t deserialize numbers, because
// everything arrives as strings. If you hit that, drop flatten and make PageParams
// its own AppQuery<PageParams> extractor, with the filters in a second query
// struct.
/// Flatten into a query struct: `#[serde(flatten)] page: PageParams`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct PageParams {
    limit: Option<u32>,
    pub cursor: Option<String>,
}

impl PageParams {
    pub(crate) fn limit(&self) -> i64 {
        i64::from(self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT))
    }
}
