use std::{future::Future, sync::Arc, time::Duration};

use anyhow::Context as _;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

use crate::{
    app::{AppState, RuntimeConfig},
    config::Settings,
    infra::{BlobFiles, Mailer, downloader},
};

pub mod admin;

mod app;
mod config;
mod error;
mod identity;
mod prelude;

mod db;
mod infra;
mod service;
mod telemetry;
mod transport;
mod util;

pub use telemetry::{TelemetryGuard, init_tracing};

pub async fn run() -> anyhow::Result<()> {
    let settings = Settings::load()?;
    log_settings(&settings);

    let db = open_database(&settings).await?;
    let shutdown = CancellationToken::new();
    let cleanup = db::spawn_cleanup(db.writer().clone(), shutdown.clone());

    let state = build_state(&settings, db.clone()).await?;
    service::auth::warm_decoy_hash().await?;

    let result = serve(state, &settings, shutdown_signal()).await;

    tracing::info!("server stopped, cleaning up");
    shutdown.cancel();
    if let Err(error) = cleanup.await {
        tracing::warn!(%error, "cleanup task failed");
    }
    checkpoint(db.writer()).await;
    tracing::info!("shutdown complete");
    result
}

/// One line describing the effective configuration, so "what was this
/// instance running with?" is answerable from the logs alone. Never add
/// secrets here (SMTP password, tokens): only facts safe to ship to a log
/// aggregator.
fn log_settings(settings: &Settings) {
    tracing::info!(
        app_env = std::env::var("APP_ENV").unwrap_or_else(|_| "local".to_owned()),
        data_dir = %settings.data_dir.display(),
        public_url = %settings.public_base(),
        host = %settings.server.host,
        port = settings.server.port,
        database = %settings.database.url,
        db_read_connections = settings.database.read_connections,
        csp = ?settings.server.csp,
        db_slow_query_ms = settings.database.slow_query_ms,
        smtp_configured = settings.mail.smtp.is_some(),
        trusted_proxies = settings.server.trusted_proxies.len(),
        "configuration loaded"
    );
}

async fn open_database(settings: &Settings) -> anyhow::Result<db::Database> {
    let db = db::Database::open(&settings.database)
        .await
        .context("unable to create database")?;
    db::migrate(db.writer())
        .await
        .context("unable to migrate database")?;
    Ok(db)
}

async fn build_state(settings: &Settings, db: db::Database) -> anyhow::Result<AppState> {
    let blobs = BlobFiles::open(settings.data_dir.join("blobs"), 16 * 1024)
        .await
        .context("cannot create blob store")?;
    let fetch_client = downloader::client_builder()
        .user_agent(concat!(
            "Mozilla/5.0 (X11; Linux x86_64; rv:157.0) Gecko/20100101 Firefox/157.0 ",
            "web-archiver/",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(Duration::from_secs(30))
        .build()
        .context("unable to create HTTP client")?;
    let mailer =
        Arc::new(Mailer::from_settings(&settings.mail).context("invalid mail configuration")?);
    let config = Arc::new(RuntimeConfig {
        public_url: settings.public_base().to_owned(),
        secure_cookie: settings.server.use_secure_cookie(&settings.public_url),
        trusted_proxies: settings.server.trusted_proxies.clone(),
        csp: settings.server.csp,
    });

    Ok(AppState {
        db,
        blobs,
        fetch_client,
        mailer,
        config,
    })
}

async fn serve(
    state: AppState,
    settings: &Settings,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(settings.server.bind_addr())
        .await
        .context("unable to create listener")?;
    let local_addr = listener
        .local_addr()
        .context("cannot get local address from listener")?;
    tracing::info!(%local_addr, "server listening");

    axum::serve(
        listener,
        app::app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .context("server error")
}

async fn checkpoint(db: &SqlitePool) {
    if let Err(error) = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(db)
        .await
    {
        tracing::warn!(%error, "WAL checkpoint failed");
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to install Ctrl+C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(%error, "failed to install SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    let signal = tokio::select! {
        () = ctrl_c => "SIGINT",
        () = terminate => "SIGTERM",
    };

    tracing::info!(signal, "shutdown signal received");
}
