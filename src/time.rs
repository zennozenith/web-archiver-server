use chrono::{DateTime, Utc};

pub(crate) const HOUR_MS: i64 = 3_600_000;
pub(crate) const DAY_MS: i64 = 86_400_000;

/// The schema stores every timestamp as INTEGER Unix epoch *milliseconds* (UTC).
pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
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

#[cfg_attr(
    not(feature = "timeago"),
    expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "relative-time formatting operates on small, bounded durations"
    )
)]
pub(crate) fn ago_timestamp(timestamp: i64) -> String {
    let Some(target) = DateTime::from_timestamp(timestamp, 0) else {
        return "invalid timestamp".to_owned();
    };

    let now = Utc::now();

    // Code compiled ONLY when the `timeago` feature is enabled
    #[cfg(feature = "timeago")]
    {
        use std::cmp::Ordering;

        let formatter = timeago::Formatter::new();
        match target.cmp(&now) {
            Ordering::Less => {
                // Target is in the past (elapsed duration)
                let duration = now
                    .signed_duration_since(target)
                    .to_std()
                    .unwrap_or_default();
                formatter.convert(duration)
            }
            Ordering::Greater => {
                // Target is in the future
                let duration = target
                    .signed_duration_since(now)
                    .to_std()
                    .unwrap_or_default();
                let text = formatter.convert(duration);
                // Adjust output for future timestamps (e.g., "3 days ago" -> "in 3 days")
                format!("in {}", text.replace(" ago", ""))
            }
            Ordering::Equal => "just now".to_owned(),
        }
    }

    // Fallback code compiled when the `timeago` feature is NOT enabled
    #[cfg(not(feature = "timeago"))]
    {
        const UNITS: &[(&str, i64)] = &[("day", 86400), ("hour", 3600), ("minute", 60)];

        let secs = target.signed_duration_since(now).num_seconds();
        let abs_secs = secs.abs();

        for &(unit, threshold) in UNITS {
            if abs_secs >= threshold {
                let val = (secs as f64 / threshold as f64).round() as i64;
                return format_relative(val, unit);
            }
        }

        "just now".to_owned()
    }
}

#[cfg(not(feature = "timeago"))]
fn format_relative(val: i64, unit: &str) -> String {
    let abs_val = val.abs();
    let suffix = if val < 0 { "ago" } else { "from now" };
    let plural = if abs_val == 1 { "" } else { "s" };
    format!("{abs_val} {unit}{plural} {suffix}")
}
