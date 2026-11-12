use sqlx::SqlitePool;

use crate::{
    auth::ClientInfo,
    db::{self, auth::NewLoginAttempt},
    error::AppError,
};

/// Records login attempts for one `login` call. Logging is best effort: a
/// failed insert is warned about but never changes the login outcome.
pub(super) struct AttemptLog<'a> {
    pool: &'a SqlitePool,
    identifier: &'a str,
    ip: String, // converted once
    user_agent: &'a str,
}

impl<'a> AttemptLog<'a> {
    pub(super) fn new(pool: &'a SqlitePool, identifier: &'a str, client: &'a ClientInfo) -> Self {
        Self {
            pool,
            identifier,
            ip: client.ip.to_string(),
            user_agent: &client.user_agent,
        }
    }

    /// Records the failure and hands the error back, for `return Err(log.failed(..).await)`.
    pub(super) async fn failed(
        &self,
        user_id: Option<i64>,
        reason: &str,
        error: AppError,
    ) -> AppError {
        self.record(user_id, Some(reason)).await;
        error
    }

    pub(super) async fn succeeded(&self, user_id: i64) {
        self.record(Some(user_id), None).await;
    }

    async fn record(&self, user_id: Option<i64>, failure_reason: Option<&str>) {
        let res = db::auth::insert_login_attempt(
            self.pool,
            NewLoginAttempt {
                identifier: self.identifier,
                user_id,
                ip: &self.ip,
                user_agent: self.user_agent,
                failure_reason,
            },
        )
        .await;
        if let Err(e) = res {
            tracing::warn!("failed to record login attempt: {e}");
        }
    }
}
