use std::time::Duration;

/// Failed password attempts before the account is locked.
pub(crate) const MAX_FAILED_ATTEMPTS: i64 = 5;
/// How long a locked account stays locked.
pub(crate) const LOCKOUT: Duration = Duration::from_secs(15 * 60);
