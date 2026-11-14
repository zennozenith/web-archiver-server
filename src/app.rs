use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRef, State},
    http::{HeaderName, StatusCode},
    middleware::from_fn,
    routing::get,
};
use sqlx::SqlitePool;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{
    infra::{BlobFiles, Mailer},
    transport::{api, asset, security, web},
    util::net::Cidr,
};

/// Small default; upload routes opt into a larger limit explicitly.
const DEFAULT_REQUEST_BYTES: usize = 2 * 1024 * 1024;
/// Multipart page forms, which may carry a thumbnail.
pub(crate) const UPLOAD_REQUEST_BYTES: usize = 16 * 1024 * 1024;
/// A full capture plus its screenshot and rewritten document.
pub(crate) const CAPTURE_REQUEST_BYTES: usize = 64 * 1024 * 1024;
/// Generous enough for archiving a page and its sub-resources.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_CONCURRENT_REQUESTS: usize = 512;

const fn request_id_header() -> HeaderName {
    HeaderName::from_static("x-request-id")
}

/// Runtime configuration copied once at startup. Keeping it behind one `Arc`
/// means a new setting only touches this struct, not every `AppState` field.
#[derive(Debug)]
pub(crate) struct ServerConfig {
    /// Normalized public base URL, without a trailing slash.
    pub public_url: String,
    pub secure_cookie: bool,
    pub trusted_proxies: Vec<Cidr>,
}

#[derive(Debug, Clone, FromRef)]
pub struct AppState {
    pub db: SqlitePool,
    pub blobs: BlobFiles,
    /// Shared client for fetching archived sub-resources.
    pub fetch_client: reqwest::Client,

    #[from_ref(skip)]
    pub(crate) mailer: Arc<Mailer>,
    #[from_ref(skip)]
    pub(crate) server: Arc<ServerConfig>,
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> (StatusCode, &'static str) {
    match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.db)
        .await
    {
        Ok(_) => (StatusCode::OK, "ok"),
        Err(e) => {
            tracing::warn!("readiness check failed: {e}");
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
        }
    }
}

pub(super) fn app(state: AppState) -> Router {
    // Probes must answer even when the request pool is saturated.
    let probes = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state.clone());

    let request_id = request_id_header();
    let app = Router::new()
        .nest("/api", api::router())
        .merge(web::router())
        .merge(asset::router())
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(ConcurrencyLimitLayer::new(MAX_CONCURRENT_REQUESTS))
        .layer(DefaultBodyLimit::max(DEFAULT_REQUEST_BYTES))
        .with_state(state);

    probes.merge(app).layer(from_fn(security::headers))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use sqlx::sqlite::SqlitePoolOptions;
    use tower::ServiceExt as _;

    use super::*;
    use crate::{
        config::MailSettings,
        db,
        infra::{BlobFiles, Mailer, downloader},
        util::time::now_ms,
    };

    async fn test_state() -> AppState {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        db::migrate(&db).await.expect("migrations");

        let blob_dir = std::env::temp_dir().join(format!(
            "web-archiver-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let blobs = BlobFiles::open(blob_dir, 1024).await.expect("blob store");
        let fetch_client = downloader::client_builder().build().expect("http client");
        let mailer = Arc::new(Mailer::from_settings(&MailSettings::default()).expect("mailer"));
        let server = Arc::new(ServerConfig {
            public_url: "http://localhost".to_owned(),
            secure_cookie: false,
            trusted_proxies: Vec::new(),
        });

        AppState {
            db,
            blobs,
            fetch_client,
            mailer,
            server,
        }
    }

    #[tokio::test]
    async fn healthz_answers_without_authentication() {
        let response = app(test_state().await)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unauthenticated_api_requests_get_401() {
        let response = app(test_state().await)
            .oneshot(
                Request::builder()
                    .uri("/api/me")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["error"]["code"], "unauthorized");
    }
}
