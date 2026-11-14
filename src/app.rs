use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRef},
    http::StatusCode,
    routing::get,
};
use sqlx::SqlitePool;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};

use crate::{
    http::{api, asset, web},
    infra::{BlobStore, Mailer},
};

/// Hard cap for a whole request body. Thumbnails and capture HTML are the
/// largest legitimate uploads.
const MAX_REQUEST_BYTES: usize = 40 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENT_REQUESTS: usize = 512;

#[derive(Debug, Clone, FromRef)]
pub struct AppState {
    pub db: SqlitePool,
    pub store: BlobStore,
    /// Shared client for fetching archived sub-resources.
    pub http: reqwest::Client,

    #[from_ref(skip)]
    pub(crate) mailer: Arc<Mailer>,
    #[from_ref(skip)]
    pub(crate) public_url: Arc<str>,
    #[from_ref(skip)]
    pub(crate) trusted_proxy: bool,
    #[from_ref(skip)]
    pub(crate) secure_cookie: bool,
}

async fn healthz() -> &'static str {
    "ok"
}

pub(super) fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .nest("/api", api::router())
        .merge(web::router())
        .merge(asset::router())
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(ConcurrencyLimitLayer::new(MAX_CONCURRENT_REQUESTS))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(state)
}
