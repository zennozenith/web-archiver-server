use std::time::Duration;

use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

use crate::config::DatabaseSettings;

pub(crate) mod archive;
pub(crate) mod capture;
pub(crate) mod collection;
pub(crate) mod content;
pub(crate) mod helper;
pub(crate) mod link;
pub(crate) mod page;
pub(crate) mod pending;
pub(crate) mod stats;
pub(crate) mod store;
pub(crate) mod tag;

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

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}
