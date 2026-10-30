use chrono::{DateTime, Utc};

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

#[deprecated(since = "0.0.0", note = "use `ago_timestamp` with a Unix timestamp")]
mod deprecated {
    use chrono::{DateTime, Utc};

    #[expect(dead_code, reason = "legacy ISO-8601 formatter kept for reference")]
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "relative-time formatting operates on small, bounded durations"
    )]
    pub(super) fn ago(iso: &str) -> Result<String, Box<dyn std::error::Error>> {
        const UNITS: &[(&str, i64)] = &[("day", 86400), ("hour", 3600), ("minute", 60)];

        // Parse the ISO string into a DateTime
        let target = DateTime::parse_from_rfc3339(iso)?;
        let now = Utc::now();

        // Calculate difference in seconds (negative if in the past)
        let secs = target.signed_duration_since(now).num_seconds();
        let abs_secs = secs.abs();

        for &(unit, threshold) in UNITS {
            if abs_secs >= threshold {
                let val = (secs as f64 / threshold as f64).round() as i64;
                return Ok(format_relative(val, unit));
            }
        }

        Ok("just now".to_owned())
    }

    fn format_relative(val: i64, unit: &str) -> String {
        let abs_val = val.abs();
        let suffix = if val < 0 { "ago" } else { "from now" };
        let plural = if abs_val == 1 { "" } else { "s" };
        format!("{abs_val} {unit}{plural} {suffix}")
    }
}
