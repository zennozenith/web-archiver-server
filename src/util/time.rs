use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};

pub(crate) const HOUR: Duration = Duration::from_secs(60 * 60);
pub(crate) const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// The schema stores every timestamp as INTEGER Unix epoch *milliseconds* (UTC).
pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Today's UTC calendar date. Use [`now_ms`] for timestamps.
pub(crate) fn today() -> NaiveDate {
    Utc::now().date_naive()
}

/// A duration in the integer milliseconds stored by the schema.
pub(crate) fn duration_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// `days` expressed in the integer milliseconds stored by the schema.
pub(crate) fn days_ms(days: i64) -> i64 {
    duration_ms(DAY).saturating_mul(days)
}

/// Timestamp arithmetic that cannot overflow (or panic in debug builds).
pub(crate) fn add_ms(ts: i64, delta: Duration) -> i64 {
    ts.saturating_add(duration_ms(delta))
}

pub(crate) fn ms_to_dt(ms: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default()
}

pub(crate) fn opt_ms_to_dt(ms: Option<i64>) -> Option<DateTime<Utc>> {
    ms.map(ms_to_dt)
}
