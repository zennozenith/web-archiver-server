use serde::Deserialize;
use serde_aux::field_attributes::deserialize_number_from_string;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct Settings {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default)]
    pub database: DatabaseSettings,

    pub server: ServerSettings,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct DatabaseSettings {
    #[serde(default = "default_db_url")]
    pub url: Box<str>,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_busy_timeout_sec")]
    pub busy_timeout_sec: u64,
}

impl Default for DatabaseSettings {
    fn default() -> Self {
        Self {
            url: default_db_url(),
            max_connections: default_max_connections(),
            busy_timeout_sec: default_busy_timeout_sec(),
        }
    }
}

impl DatabaseSettings {
    pub(crate) fn connect_options(&self) -> SqliteConnectOptions {
        SqliteConnectOptions::from_str(&self.url)
            .expect("Invalid database url")
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(self.busy_timeout_sec))
        // .optimize_on_close(true)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ServerSettings {
    pub host: Option<String>,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub port: u16,
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("data")
}

fn default_db_url() -> Box<str> {
    String::from("sqlite://data/db/app.db").into_boxed_str()
}

const fn default_max_connections() -> u32 {
    10
}

const fn default_busy_timeout_sec() -> u64 {
    5
}

impl Settings {
    pub(crate) fn load() -> Self {
        let env = std::env::var("APP_ENV").unwrap_or_else(|_| "local".into());
        config::Config::builder()
            // .add_source(config::File::with_name("config/base"))
            .add_source(config::File::with_name(&format!("config/{env}")).required(false))
            // APP_SERVER__PORT=8080 -> server.port
            .add_source(
                config::Environment::with_prefix("APP")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()
            .expect("Unable to load config")
            .try_deserialize()
            .expect("Unable to load config")
    }
}
