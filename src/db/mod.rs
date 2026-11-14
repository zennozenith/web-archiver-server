use std::time::Duration;

use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{
    config::DatabaseSettings,
    util::time::{days_ms, now_ms},
};

pub(crate) mod api_key;
pub(crate) mod archive;
pub(crate) mod audit;
pub(crate) mod auth;
pub(crate) mod blob;
pub(crate) mod capture;
pub(crate) mod collection;
pub(crate) mod content;
pub(crate) mod create;
pub(crate) mod cursor;
pub(crate) mod group;
pub(crate) mod ids;
pub(crate) mod page;
pub(crate) mod patch;
pub(crate) mod pending;
pub(crate) mod permission;
pub(crate) mod profile;
pub(crate) mod role;
pub(crate) mod session;
pub(crate) mod stats;
pub(crate) mod tag;
pub(crate) mod user;

pub(crate) async fn create_pool(
    database_settings: &DatabaseSettings,
) -> Result<SqlitePool, sqlx::Error> {
    // `create_if_missing` creates the file, not its parent directory.
    let options = database_settings.connect_options()?;
    if let Some(parent) = DatabaseSettings::parent_dir(&options) {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(sqlx::Error::Io)?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(database_settings.max_connections)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(10 * 60))
        .max_lifetime(Duration::from_secs(30 * 60))
        .connect_with(options)
        .await?;

    Ok(pool)
}

/// Apply all migrations in ./migrations (embedded into the binary at compile time).
pub(crate) async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

/// Hourly housekeeping, taken from the "OPERATIONAL NOTES" in the schema file.
///
/// The returned handle stops when `shutdown` is cancelled.
pub(crate) fn spawn_cleanup(pool: SqlitePool, shutdown: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                _ = tick.tick() => {}
            }
            let now = now_ms();
            let cutoff = |days: i64| now.saturating_sub(days_ms(days));

            let jobs = [
                ("DELETE FROM sessions WHERE expires_at < ?", cutoff(30)),
                ("DELETE FROM auth_tokens WHERE expires_at < ?", cutoff(7)),
                (
                    "DELETE FROM login_attempts WHERE created_at < ?",
                    cutoff(90),
                ),
            ];
            for (sql, ts) in jobs {
                run_cleanup(&pool, sql, ts).await;
            }
            let _throw_away = sqlx::query("PRAGMA optimize").execute(&pool).await;
        }
    })
}

async fn run_cleanup(pool: &SqlitePool, sql: &'static str, ts: i64) {
    match sqlx::query(sql).bind(ts).execute(pool).await {
        Ok(r) => {
            let removed = r.rows_affected();
            if removed > 0 {
                tracing::info!("cleanup: {removed} rows removed by `{sql}`");
            }
        }
        Err(e) => tracing::warn!("cleanup failed for `{sql}`: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn all_migrations_apply_cleanly() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        migrate(&pool).await.expect("migrations apply");

        let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .expect("migration table");
        assert!(applied > 0, "expected at least one applied migration");
    }
}
