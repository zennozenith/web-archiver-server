#[cfg(debug_assertions)]
use std::sync::Arc;
use std::{net::SocketAddr, time::Duration};

use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

use crate::{
    app::AppState,
    config::Settings,
    infra::{BlobStore, Mailer},
};

mod app;
mod auth;
mod config;
mod error;
mod prelude;

mod db;
mod http;
mod infra;
mod service;
mod util;

#[expect(clippy::too_many_lines, reason = "")]
#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!(
                    "{}=debug,tower_http=info,sqlx=warn",
                    env!("CARGO_CRATE_NAME")
                )
                .into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let settings = Settings::load();
    let store = BlobStore::open(settings.data_dir.join("blobs"), 16 * 1024)
        .await
        .expect("Cannot create blob store");

    let db = db::create_pool(&settings.database)
        .await
        .expect("Unable to create database");
    db::migrate(&db).await.expect("Unable to migrate database");
    db::spawn_cleanup(db.clone());

    let http = reqwest::Client::builder()
        .user_agent(concat!(
            "Mozilla/5.0 (X11; Linux x86_64; rv:157.0) Gecko/20100101 Firefox/157.0 ",
            "web-archiver/",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .expect("Unable to create HTTP client");

    // let mailer: Arc<dyn Mailer> = match &settings.mail.smtp {
    //     Some(smtp) => Arc::new(
    //         SmtpMailer::new(&settings.mail.from, smtp).expect("invalid mail configuration"),
    //     ),
    //     None if cfg!(debug_assertions) => Arc::new(LogMailer),
    //     None => panic!("mail.smtp must be configured in release builds"),
    // };

    #[cfg(debug_assertions)]
    let mailer = Arc::new(Mailer::Log(infra::LogMailer));

    let state = AppState {
        db,
        store,
        http,
        mailer,
        public_url: Arc::from("https://example.com"), // TODO: from settings
    };
    service::auth::init().await;

    let app = app::app(state);

    tracing::debug!("{:#?}", &settings);

    let listener = tokio::net::TcpListener::bind(format!(
        "{}:{}",
        settings.server.host.as_deref().unwrap_or("127.0.0.1"),
        settings.server.port
    ))
    .await
    .expect("Unable to create listener");

    tracing::info!(
        "listening on http://{}",
        listener
            .local_addr()
            .expect("Cannot get local address from listener")
            .to_string()
    );

    let _err = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await;
}
