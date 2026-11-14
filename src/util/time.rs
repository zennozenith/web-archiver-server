use chrono::{DateTime, NaiveDate, Utc};

pub(crate) const HOUR_MS: i64 = 3_600_000;
pub(crate) const DAY_MS: i64 = 86_400_000;

/// The schema stores every timestamp as INTEGER Unix epoch *milliseconds* (UTC).
pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Today's UTC calendar date. Use [`now_ms`] for timestamps.
pub(crate) fn today() -> NaiveDate {
    Utc::now().date_naive()
}

/// Timestamp arithmetic that cannot overflow (or panic in debug builds).
pub(crate) const fn add_ms(ts: i64, delta: i64) -> i64 {
    ts.saturating_add(delta)
}

pub(crate) fn ms_to_dt(ms: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default()
}

pub(crate) fn opt_ms_to_dt(ms: Option<i64>) -> Option<DateTime<Utc>> {
    ms.map(ms_to_dt)
}
