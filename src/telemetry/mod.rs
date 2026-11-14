//! Process-wide logging: filtering, output formats, rotating log files and a
//! panic hook that routes panics through `tracing`.
//!
//! Logging is configured from the environment rather than from
//! [`Settings`](crate::config::Settings) because it must be running *before*
//! the configuration is loaded, so that a broken config file is itself logged.
//!
//! | Variable           | Default                          | Meaning                                              |
//! |--------------------|----------------------------------|------------------------------------------------------|
//! | `RUST_LOG`         | see [`default_filter`]           | `tracing-subscriber` filter directives               |
//! | `APP_LOG__FORMAT`  | `pretty` (debug) / `json` (release) | stdout format: `pretty`, `compact` or `json`      |
//! | `APP_LOG__DIR`     | unset                            | also write JSON logs to daily-rotated files here     |
//! | `APP_LOG__MAX_FILES` | `14`                           | rotated files to keep when `APP_LOG__DIR` is set     |
//! | `NO_COLOR`         | unset                            | disable ANSI colours on a terminal                   |

pub(crate) mod redact;

use std::{
    backtrace::{Backtrace, BacktraceStatus},
    env,
    error::Error,
    io::IsTerminal as _,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Context as _, ensure};
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{
    EnvFilter, Layer as _, Registry, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

const FORMAT_VAR: &str = "APP_LOG__FORMAT";
const DIR_VAR: &str = "APP_LOG__DIR";
const MAX_FILES_VAR: &str = "APP_LOG__MAX_FILES";
const DEFAULT_MAX_FILES: usize = 14;

/// Rotated files are named `web-archiver.<date>.log`.
const LOG_FILE_PREFIX: &str = "web-archiver";
const LOG_FILE_SUFFIX: &str = "log";

type BoxedLayer = Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogFormat {
    /// Multi-line, coloured output for local development.
    Pretty,
    /// One human-readable line per event.
    Compact,
    /// One JSON object per event, for log aggregation.
    Json,
}

impl LogFormat {
    const fn default_for_build() -> Self {
        if cfg!(debug_assertions) {
            Self::Pretty
        } else {
            Self::Json
        }
    }
}

impl FromStr for LogFormat {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pretty" => Ok(Self::Pretty),
            "compact" | "text" => Ok(Self::Compact),
            "json" => Ok(Self::Json),
            other => Err(format!(
                "unknown log format `{other}` (expected `pretty`, `compact` or `json`)"
            )),
        }
    }
}

#[derive(Debug)]
struct LogConfig {
    format: LogFormat,
    /// When set, JSON logs are also written to rotating files in this directory.
    dir: Option<PathBuf>,
    max_files: usize,
    /// Colour only when attached to a terminal and `NO_COLOR` is not set.
    ansi: bool,
}

impl LogConfig {
    fn from_env() -> anyhow::Result<Self> {
        let format = env::var(FORMAT_VAR)
            .ok()
            .map(|raw| raw.parse::<LogFormat>())
            .transpose()
            .map_err(|e| anyhow::anyhow!("{FORMAT_VAR}: {e}"))?
            .unwrap_or_else(LogFormat::default_for_build);

        let dir = env::var_os(DIR_VAR)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);

        let max_files = match env::var(MAX_FILES_VAR) {
            Ok(raw) => raw
                .trim()
                .parse::<usize>()
                .with_context(|| format!("{MAX_FILES_VAR} must be a positive integer"))?,
            Err(_) => DEFAULT_MAX_FILES,
        };
        ensure!(max_files > 0, "{MAX_FILES_VAR} must be at least 1");

        let ansi = std::io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none();

        Ok(Self {
            format,
            dir,
            max_files,
            ansi,
        })
    }
}

/// Keeps the background log writer alive.
///
/// Hold it for the whole life of the program. Dropping it flushes buffered
/// file output, and dropping it *early* silently loses log lines, so never
/// bind it to a bare `_`.
#[derive(Debug)]
#[must_use = "dropping the guard immediately stops file logging and can lose buffered lines"]
pub struct TelemetryGuard {
    _file_writer: Option<WorkerGuard>,
}

/// Install the global `tracing` subscriber and the panic hook.
///
/// Fails when the configuration is invalid or the log directory is not
/// writable. A server that cannot write its logs should refuse to start
/// rather than run blind.
pub fn init_tracing() -> anyhow::Result<TelemetryGuard> {
    let config = LogConfig::from_env()?;

    let mut layers: Vec<BoxedLayer> = vec![stdout_layer(&config)];
    let file_writer = match &config.dir {
        Some(dir) => {
            let (layer, guard) = file_layer(dir, config.max_files)?;
            layers.push(layer);
            Some(guard)
        }
        None => None,
    };

    tracing_subscriber::registry()
        .with(layers)
        // Outermost, so it gates every layer above it.
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| default_filter()))
        .try_init()
        .context("a global tracing subscriber is already installed")?;

    install_panic_hook();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        profile = if cfg!(debug_assertions) { "debug" } else { "release" },
        log_format = ?config.format,
        log_dir = config.dir.as_ref().map(|dir| dir.display().to_string()),
        "logging initialised"
    );

    Ok(TelemetryGuard {
        _file_writer: file_writer,
    })
}

/// Used when `RUST_LOG` is unset or invalid. The application logs at `debug`
/// in debug builds and `info` in release builds; chatty dependencies stay at
/// `warn`. `sqlx=warn` still lets slow statements through (they are emitted at
/// `warn`), while per-statement logging stays off until asked for with
/// `RUST_LOG=sqlx::query=debug`.
fn default_filter() -> EnvFilter {
    let level = if cfg!(debug_assertions) {
        "debug"
    } else {
        "info"
    };
    EnvFilter::new(format!(
        "{crate_name}={level},tower_http=info,sqlx=warn,hyper=warn,hyper_util=warn,\
         h2=warn,reqwest=warn,rustls=warn,lettre=warn",
        crate_name = env!("CARGO_CRATE_NAME"),
    ))
}

fn stdout_layer(config: &LogConfig) -> BoxedLayer {
    let layer = fmt::layer::<Registry>()
        .with_ansi(config.ansi)
        .with_target(true);
    match config.format {
        LogFormat::Pretty => layer.pretty().boxed(),
        LogFormat::Compact => layer.compact().boxed(),
        LogFormat::Json => json_layer(std::io::stdout),
    }
}

/// Daily-rotated JSON file, written from a background thread so disk I/O never
/// blocks a request. The writer is *lossy*: if the disk stalls and the buffer
/// fills, lines are dropped instead of stalling the server.
fn file_layer(dir: &Path, max_files: usize) -> anyhow::Result<(BoxedLayer, WorkerGuard)> {
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(max_files)
        .build(dir)
        .with_context(|| format!("cannot open log directory {}", dir.display()))?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    Ok((json_layer(writer), guard))
}

/// One JSON object per event.
///
/// `with_span_list(true)` is deliberate: it lists *every* enclosing span, so
/// the `request_id` and `user_id` of the `http_request` span appear on lines
/// logged from nested spans too. Showing only the current span would lose
/// them as soon as a child span is entered.
fn json_layer<W>(writer: W) -> BoxedLayer
where
    W: for<'writer> fmt::MakeWriter<'writer> + Send + Sync + 'static,
{
    fmt::layer::<Registry>()
        .json()
        .with_writer(writer)
        .flatten_event(true)
        .with_current_span(false)
        .with_span_list(true)
        .with_target(true)
        .boxed()
}

/// Route panics through `tracing`, so they reach the same sinks (and carry the
/// same span context) as every other event instead of landing as unstructured
/// text on stderr.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        let location = info
            .location()
            .map_or_else(|| "unknown".to_owned(), |l| format!("{}:{}", l.file(), l.line()));

        let backtrace = Backtrace::capture();
        if backtrace.status() == BacktraceStatus::Captured {
            tracing::error!(%message, %location, %backtrace, "thread panicked");
        } else {
            tracing::error!(%message, %location, "thread panicked");
        }
    }));
}

/// Render an error and every error it wraps as `outer: inner: root`.
///
/// `Display` alone drops the cause chain (`database error` says nothing about
/// *why*), and `Debug` is noisy. Causes whose text the parent already ends
/// with are skipped, because `thiserror`'s `#[from]` often repeats them.
pub(crate) fn error_chain(error: &(dyn Error + 'static)) -> String {
    let mut out = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !out.ends_with(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        source = cause.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use super::*;

    #[test]
    fn log_format_parses_known_names_case_insensitively() {
        assert_eq!("json".parse::<LogFormat>(), Ok(LogFormat::Json));
        assert_eq!(" Pretty ".parse::<LogFormat>(), Ok(LogFormat::Pretty));
        assert_eq!("COMPACT".parse::<LogFormat>(), Ok(LogFormat::Compact));
        assert_eq!("text".parse::<LogFormat>(), Ok(LogFormat::Compact));
    }

    #[test]
    fn log_format_rejects_unknown_names() {
        let error = "xml".parse::<LogFormat>().expect_err("xml is not a format");
        assert!(error.contains("xml"), "message names the bad value: {error}");
    }

    #[derive(Debug)]
    struct Outer(Inner);
    #[derive(Debug)]
    struct Inner;

    impl fmt::Display for Outer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("outer")
        }
    }
    impl fmt::Display for Inner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("inner")
        }
    }
    impl Error for Inner {}
    impl Error for Outer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn error_chain_lists_every_cause() {
        assert_eq!(error_chain(&Outer(Inner)), "outer: inner");
    }
}
