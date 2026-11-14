use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

use anyhow::{Context as _, ensure};
use secrecy::SecretString;
use serde::Deserialize;
use serde_aux::field_attributes::deserialize_number_from_string;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use url::Url;

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct Settings {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default)]
    pub database: DatabaseSettings,
    #[serde(default)]
    pub mail: MailSettings,

    pub public_url: Box<str>,
    pub server: ServerSettings,
}

impl Settings {
    pub(crate) fn load() -> anyhow::Result<Self> {
        let env = std::env::var("APP_ENV").unwrap_or_else(|_| "local".into());
        let settings: Self = config::Config::builder()
            .add_source(config::File::with_name(&format!("config/{env}")).required(false))
            // APP_SERVER__PORT=8080 -> server.port
            .add_source(
                config::Environment::with_prefix("APP")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()
            .context("unable to load configuration")?
            .try_deserialize()
            .context("unable to deserialize configuration")?;

        settings.validate()?;
        Ok(settings)
    }

    fn validate(&self) -> anyhow::Result<()> {
        let public_url = Url::parse(&self.public_url)
            .with_context(|| format!("invalid public_url `{}`", self.public_url))?;
        ensure!(
            matches!(public_url.scheme(), "http" | "https"),
            "public_url must use http or https"
        );
        ensure!(
            public_url.host_str().is_some_and(|host| !host.is_empty()),
            "public_url must include a host"
        );
        ensure!(
            public_url.username().is_empty() && public_url.password().is_none(),
            "public_url must not contain credentials"
        );

        if self.mail.smtp.is_some() {
            ensure!(
                self.mail.from.is_some(),
                "mail.from must be configured when mail.smtp is set"
            );
        }
        Ok(())
    }

    /// `public_url` without a trailing slash, ready to concatenate paths onto.
    pub(crate) fn public_url(&self) -> String {
        self.public_url.trim_end_matches('/').to_owned()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct DatabaseSettings {
    #[serde(default = "default_db_url")]
    pub url: Box<str>,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_busy_timeout_secs")]
    pub busy_timeout_secs: u64,
}

impl Default for DatabaseSettings {
    fn default() -> Self {
        Self {
            url: default_db_url(),
            max_connections: default_max_connections(),
            busy_timeout_secs: default_busy_timeout_secs(),
        }
    }
}

impl DatabaseSettings {
    pub(crate) fn connect_options(&self) -> sqlx::Result<SqliteConnectOptions> {
        Ok(SqliteConnectOptions::from_str(&self.url)?
            .create_if_missing(true)
            .foreign_keys(true) // PRAGMA foreign_keys = ON
            .journal_mode(SqliteJournalMode::Wal) // PRAGMA journal_mode = WAL
            .synchronous(SqliteSynchronous::Normal) // PRAGMA synchronous = NORMAL
            .busy_timeout(Duration::from_secs(self.busy_timeout_secs))) // PRAGMA busy_timeout = ...
    }
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ServerSettings {
    #[serde(default = "default_host")]
    pub host: IpAddr,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub port: u16,
    /// Honour `X-Forwarded-For` / `X-Real-IP`. Only enable behind a reverse proxy
    /// that overwrites those headers.
    #[serde(default)]
    pub trusted_proxy: bool,
    /// Add `Secure` to the session cookie. Disable only for plain-HTTP local dev.
    #[serde(default = "default_secure_cookie")]
    pub secure_cookie: bool,
}

impl ServerSettings {
    pub(crate) const fn bind_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

#[derive(Debug, Default, Deserialize, Clone)]
pub(crate) struct MailSettings {
    /// Envelope sender, e.g. `Web Archiver <noreply@example.com>`.
    pub from: Option<String>,
    /// When absent, development logs mail to the console.
    pub smtp: Option<SmtpSettings>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct SmtpSettings {
    pub host: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    pub username: String,
    pub password: SecretString,
    /// STARTTLS (587). Disable for implicit TLS on port 465.
    #[serde(default = "default_starttls")]
    pub starttls: bool,
}

const fn default_smtp_port() -> u16 {
    587
}

const fn default_starttls() -> bool {
    true
}

const fn default_host() -> IpAddr {
    IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

const fn default_secure_cookie() -> bool {
    true
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

const fn default_busy_timeout_secs() -> u64 {
    5
}
