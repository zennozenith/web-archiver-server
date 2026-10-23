use chrono::{DateTime, Utc};

pub fn ago_timestamp(timestamp: i64) -> String {
    let target = match DateTime::from_timestamp(timestamp, 0) {
        Some(dt) => dt,
        None => return "invalid timestamp".to_string(),
    };

    let now = Utc::now();

    // Code compiled ONLY when the `timeago` feature is enabled
    #[cfg(feature = "timeago")]
    {
        use std::cmp::Ordering;

        let formatter = timeago::Formatter::new();
        return match target.cmp(&now) {
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
                let formatted = formatter.convert(duration);
                // Adjust output for future timestamps (e.g., "3 days ago" -> "in 3 days")
                format!("in {}", formatted.replace(" ago", ""))
            }
            Ordering::Equal => "just now".to_string(),
        };
    }

    // Fallback code compiled when the `timeago` feature is NOT enabled
    #[cfg(not(feature = "timeago"))]
    {
        let secs = target.signed_duration_since(now).num_seconds();
        let abs_secs = secs.abs();

        const UNITS: &[(&str, i64)] = &[("day", 86400), ("hour", 3600), ("minute", 60)];

        for &(unit, threshold) in UNITS {
            if abs_secs >= threshold {
                let val = (secs as f64 / threshold as f64).round() as i64;
                let abs_val = val.abs();
                let suffix = if val < 0 { "ago" } else { "from now" };
                let plural = if abs_val == 1 { "" } else { "s" };
                return format!("{} {}{} {}", abs_val, unit, plural, suffix);
            }
        }

        "just now".to_string()
    }
}

#[deprecated]
mod deprecated {
    use chrono::{DateTime, Utc};

    #[allow(unused)]
    pub fn ago(iso: &str) -> Result<String, Box<dyn std::error::Error>> {
        // Parse the ISO string into a DateTime
        let target = DateTime::parse_from_rfc3339(iso)?;
        let now = Utc::now();

        // Calculate difference in seconds (negative if in the past)
        let secs = target.signed_duration_since(now).num_seconds();
        let abs_secs = secs.abs();

        const UNITS: &[(&str, i64)] = &[("day", 86400), ("hour", 3600), ("minute", 60)];

        for &(unit, threshold) in UNITS {
            if abs_secs >= threshold {
                let val = (secs as f64 / threshold as f64).round() as i64;
                return Ok(format_relative(val, unit));
            }
        }

        Ok("just now".to_string())
    }

    fn format_relative(val: i64, unit: &str) -> String {
        let abs_val = val.abs();
        let suffix = if val < 0 { "ago" } else { "from now" };
        let plural = if abs_val == 1 { "" } else { "s" };
        format!("{} {}{} {}", abs_val, unit, plural, suffix)
    }
}
