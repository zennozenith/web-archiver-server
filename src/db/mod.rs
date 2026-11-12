use std::time::Duration;

use chrono::Utc;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

use crate::{config::DatabaseSettings, util::time::DAY_MS};

pub(crate) mod api_key;
pub(crate) mod profile;
pub(crate) mod role;
pub(crate) mod session;
// pub(crate) mod archive;
pub(crate) mod audit;
pub(crate) mod auth;
// pub(crate) mod capture;
// pub(crate) mod collection;
// pub(crate) mod content;
// pub(crate) mod helper;
// pub(crate) mod page;
// pub(crate) mod pending;
pub(crate) mod stats;
// pub(crate) mod store;
// pub(crate) mod tag;
pub(crate) mod user;

pub(crate) async fn create_pool(
    database_settings: &DatabaseSettings,
) -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .max_connections(database_settings.max_connections)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_mins(10))
        .max_lifetime(Duration::from_mins(30))
        .connect_with(database_settings.connect_options())
        .await?;

    Ok(pool)
}

/// Apply all migrations in ./migrations (embedded into the binary at compile time).
pub(crate) async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

/// Hourly housekeeping, taken from the "OPERATIONAL NOTES" in the schema file.
pub(crate) fn spawn_cleanup(pool: SqlitePool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let now = Utc::now().timestamp_millis();
            let cutoff = |days: i64| now.saturating_sub(days.saturating_mul(DAY_MS));

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
    });
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
