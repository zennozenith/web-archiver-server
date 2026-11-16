use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

use anyhow::{Context as _, ensure};
use lettre::message::Mailbox;
use secrecy::SecretString;
use serde::{Deserialize, Deserializer, de};
use serde_aux::field_attributes::deserialize_number_from_string;
use sqlx::{
    ConnectOptions as _,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};
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

        ensure!(
            self.database.read_connections >= 1,
            "database.read_connections must be at least 1"
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
    /// Size of the read pool. The writer is always a single connection (see
    /// `db::Database`). `max_connections` is accepted for older config files.
    #[serde(default = "default_read_connections", alias = "max_connections")]
    pub read_connections: u32,
    #[serde(default = "default_busy_timeout_secs")]
    pub busy_timeout_secs: u64,
    /// Statements slower than this are logged at `WARN` with their SQL text and
    /// timings. Set it above your normal worst case so the log stays quiet
    /// until something is actually wrong.
    #[serde(default = "default_slow_query_ms")]
    pub slow_query_ms: u64,
}

impl Default for DatabaseSettings {
    fn default() -> Self {
        Self {
            url: default_db_url(),
            read_connections: default_read_connections(),
            busy_timeout_secs: default_busy_timeout_secs(),
            slow_query_ms: default_slow_query_ms(),
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
            .busy_timeout(Duration::from_secs(self.busy_timeout_secs)) // PRAGMA busy_timeout = ...
            // Every statement at DEBUG: hidden by the default filter, enable with
            // `RUST_LOG=sqlx::query=debug`. Bind values are never logged, only SQL text.
            .log_statements(log::LevelFilter::Debug)
            // Anything over the threshold is promoted to WARN, which the default
            // filter lets through.
            .log_slow_statements(
                log::LevelFilter::Warn,
                Duration::from_millis(self.slow_query_ms),
            ))
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
    /// Content-Security-Policy rollout stage; see [`CspMode`].
    #[serde(default)]
    pub csp: CspMode,
}

/// How the strict app CSP is rolled out.
#[derive(Debug, Default, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CspMode {
    /// Enforce a transitional policy (scripts may use `unsafe-inline` and
    /// `unsafe-eval`) and only *report* violations of the strict one to
    /// `/csp-report`. Switch to `enforce` once the reports have been quiet.
    #[default]
    ReportOnly,
    /// Enforce the strict policy (and keep reporting violations).
    Enforce,
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
    /// DEVELOPMENT ONLY: skip TLS certificate verification so a mock server
    /// with a self-signed certificate can be used. Never enable in production.
    #[serde(default)]
    pub accept_invalid_certs: bool,
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

/// SQLite permits one writer, which `db::Database` pins to a single
/// connection (avoiding `SQLITE_BUSY` on read-to-write upgrades). Readers are
/// cheap under WAL, so the read pool can be larger.
const fn default_read_connections() -> u32 {
    8
}

const fn default_busy_timeout_secs() -> u64 {
    5
}

const fn default_slow_query_ms() -> u64 {
    250
}
