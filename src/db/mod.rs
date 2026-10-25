use std::time::Duration;

use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

use crate::config::DatabaseSettings;

pub mod health;
pub mod helper;
pub mod pending;

pub async fn create_pool(database_settings: &DatabaseSettings) -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .max_connections(database_settings.max_connections)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(1800))
        .connect_with(database_settings.connect_options())
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}
