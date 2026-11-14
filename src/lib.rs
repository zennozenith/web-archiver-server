use std::{future::Future, sync::Arc, time::Duration};

use anyhow::Context as _;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

use crate::{
    app::{AppState, ServerConfig},
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
mod transport;
mod util;

pub fn init_tracing() {
    let default_level = if cfg!(debug_assertions) {
        "debug"
    } else {
        "info"
    };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!(
                    "{crate_name}={default_level},tower_http=info,sqlx=warn",
                    crate_name = env!("CARGO_CRATE_NAME"),
                )
                .into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}

pub async fn run() -> anyhow::Result<()> {
    let settings = Settings::load()?;
    log_settings(&settings);

    let db = open_database(&settings).await?;
    let shutdown = CancellationToken::new();
    let cleanup = db::spawn_cleanup(db.clone(), shutdown.clone());

    let state = build_state(&settings, db.clone()).await?;
    service::auth::warm_decoy_hash().await?;

    let result = serve(state, &settings, shutdown_signal()).await;

    shutdown.cancel();
    if let Err(e) = cleanup.await {
        tracing::warn!("cleanup task failed: {e}");
    }
    checkpoint(&db).await;
    result
}

fn log_settings(settings: &Settings) {
    tracing::debug!(
        data_dir = %settings.data_dir.display(),
        public_url = %settings.public_base(),
        host = %settings.server.host,
        port = settings.server.port,
        "settings loaded"
    );
}

async fn open_database(settings: &Settings) -> anyhow::Result<SqlitePool> {
    let db = db::create_pool(&settings.database)
        .await
        .context("unable to create database")?;
    db::migrate(&db)
        .await
        .context("unable to migrate database")?;
    Ok(db)
}

async fn build_state(settings: &Settings, db: SqlitePool) -> anyhow::Result<AppState> {
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
    let server = Arc::new(ServerConfig {
        public_url: settings.public_base().to_owned(),
        secure_cookie: settings.server.use_secure_cookie(&settings.public_url),
        trusted_proxies: settings.server.trusted_proxies.clone(),
    });

    Ok(AppState {
        db,
        blobs,
        fetch_client,
        mailer,
        server,
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
    tracing::info!("listening on http://{local_addr}");

    axum::serve(
        listener,
        app::app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .context("server error")
}

async fn checkpoint(db: &SqlitePool) {
    if let Err(e) = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(db)
        .await
    {
        tracing::warn!("WAL checkpoint failed: {e}");
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!("failed to install Ctrl+C handler: {e}");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(e) => tracing::error!("failed to install SIGTERM handler: {e}"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
