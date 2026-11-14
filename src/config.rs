use std::{
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use anyhow::{Context as _, ensure};
use lettre::message::Mailbox;
use secrecy::SecretString;
use serde::{Deserialize, Deserializer, de};
use serde_aux::field_attributes::deserialize_number_from_string;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use url::Url;

use crate::util::net::Cidr;

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct Settings {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default)]
    pub database: DatabaseSettings,
    #[serde(default)]
    pub mail: MailSettings,

    /// Absolute, public base URL. Stored normalized so links do not need to
    /// re-parse it at every mail.
    pub public_url: Url,
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
        ensure!(
            matches!(self.public_url.scheme(), "http" | "https"),
            "public_url must use http or https"
        );
        ensure!(
            self.public_url
                .host_str()
                .is_some_and(|host| !host.is_empty()),
            "public_url must include a host"
        );
        ensure!(
            self.public_url.username().is_empty() && self.public_url.password().is_none(),
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
    pub(crate) fn public_base(&self) -> &str {
        self.public_url.as_str().trim_end_matches('/')
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

    /// Parent directory of the database file, unless this is an in-memory URL.
    pub(crate) fn parent_dir(options: &SqliteConnectOptions) -> Option<&Path> {
        let filename = options.get_filename();
        if filename == Path::new(":memory:") {
            return None;
        }
        filename.parent().filter(|p| !p.as_os_str().is_empty())
    }
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ServerSettings {
    #[serde(default = "default_host")]
    pub host: IpAddr,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub port: u16,
    /// CIDR blocks whose socket peer address may supply a forwarded client IP.
    /// Empty means forwarding headers are ignored entirely. The only forwarding
    /// header read is `X-Forwarded-For`; its rightmost untrusted hop is used.
    #[serde(default)]
    pub trusted_proxies: Vec<Cidr>,
    /// Add `Secure` to the session cookie. When unset, follows the public URL
    /// scheme, so plain-HTTP development works without extra configuration.
    #[serde(default)]
    pub secure_cookie: Option<bool>,
}

impl ServerSettings {
    pub(crate) const fn bind_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }

    pub(crate) fn use_secure_cookie(&self, public_url: &Url) -> bool {
        self.secure_cookie
            .unwrap_or_else(|| public_url.scheme() == "https")
    }
}

#[derive(Debug, Default, Deserialize, Clone)]
pub(crate) struct MailSettings {
    /// Envelope sender, e.g. `Web Archiver <noreply@example.com>`. Parsed once
    /// at configuration load, so transports can never disagree about validity.
    #[serde(default, deserialize_with = "deserialize_optional_mailbox")]
    pub from: Option<Mailbox>,
    /// When absent, development logs mail to the console.
    pub smtp: Option<SmtpSettings>,
}

fn deserialize_optional_mailbox<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Mailbox>, D::Error> {
    let raw = Option::<String>::deserialize(deserializer)?;
    raw.map(|value| value.parse().map_err(de::Error::custom))
        .transpose()
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

fn default_data_dir() -> PathBuf {
    PathBuf::from("data")
}

fn default_db_url() -> Box<str> {
    String::from("sqlite://data/db/app.db").into_boxed_str()
}

/// SQLite permits one writer. A single pooled connection avoids
/// `SQLITE_BUSY` when a deferred transaction upgrades from read to write.
const fn default_max_connections() -> u32 {
    1
}

const fn default_busy_timeout_secs() -> u64 {
    5
}
