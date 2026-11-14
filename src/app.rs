use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRef, Request, State},
    http::{HeaderName, HeaderValue, StatusCode, header::RETRY_AFTER},
    middleware::{Next, from_fn, from_fn_with_state},
    response::{IntoResponse, Response},
    routing::get,
};
use sqlx::SqlitePool;
use tokio::sync::Semaphore;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{
    db::Database,
    infra::{BlobFiles, Mailer},
    transport::{api, asset, security, web},
    util::net::Cidr,
};

/// Small default; upload routes opt into a larger limit explicitly.
const DEFAULT_REQUEST_BYTES: usize = 2 * 1024 * 1024;
/// Multipart page forms, which may carry a thumbnail.
pub(crate) const MAX_UPLOAD_BODY_BYTES: usize = 16 * 1024 * 1024;
/// A full capture plus its screenshot and rewritten document.
pub(crate) const MAX_CAPTURE_BODY_BYTES: usize = 64 * 1024 * 1024;
/// Default per-request deadline. Long-running captures opt in to `CAPTURE_TIMEOUT`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Archiving a page and all of its sub-resources can legitimately take minutes.
pub(crate) const CAPTURE_TIMEOUT: Duration = Duration::from_secs(300);
/// `/readyz` must answer quickly even when the pool is saturated.
const READINESS_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONCURRENT_REQUESTS: usize = 512;

const fn request_id_header() -> HeaderName {
    HeaderName::from_static("x-request-id")
}

/// Runtime configuration copied once at startup. Keeping it behind one `Arc`
/// means a new setting only touches this struct, not every `AppState` field.
#[derive(Debug)]
pub(crate) struct RuntimeConfig {
    /// Normalized public base URL, without a trailing slash.
    pub public_url: String,
    pub secure_cookie: bool,
    pub trusted_proxies: Vec<Cidr>,
}

#[derive(Debug, Clone, FromRef)]
pub struct AppState {
    pub(crate) db: Database,
    pub blobs: BlobFiles,
    /// Shared client for fetching archived sub-resources.
    pub fetch_client: reqwest::Client,

    #[from_ref(skip)]
    pub(crate) mailer: Arc<Mailer>,
    #[from_ref(skip)]
    pub(crate) config: Arc<RuntimeConfig>,
}

/// Extractors that ask for a bare `SqlitePool` get the single-writer pool.
impl FromRef<AppState> for SqlitePool {
    fn from_ref(state: &AppState) -> Self {
        state.db.writer().clone()
    }
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> (StatusCode, &'static str) {
    let probe = sqlx::query_scalar::<_, i64>("SELECT 1").fetch_one(state.db.reader());
    match tokio::time::timeout(READINESS_TIMEOUT, probe).await {
        Ok(Ok(_)) => (StatusCode::OK, "ok"),
        Ok(Err(e)) => {
            tracing::warn!("readiness check failed: {e}");
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
        }
        Err(_) => {
            tracing::warn!("readiness check timed out");
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
        }
    }
}

/// Shed excess requests instead of letting them queue indefinitely. Probes are
/// mounted outside this layer so they keep answering under saturation.
async fn shed_requests(
    State(semaphore): State<Arc<Semaphore>>,
    req: Request,
    next: Next,
) -> Response {
    let Ok(permit) = semaphore.try_acquire_owned() else {
        let mut response = (StatusCode::SERVICE_UNAVAILABLE, "server busy").into_response();
        response
            .headers_mut()
            .insert(RETRY_AFTER, HeaderValue::from_static("1"));
        return response;
    };

    let response = next.run(req).await;
    drop(permit);
    response
}

pub(super) fn app(state: AppState) -> Router {
    // Probes must answer even when the request pool is saturated.
    let probes = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state.clone());

    // Normal routes share the short default deadline.
    let normal = Router::new()
        .nest("/api", api::router())
        .merge(web::router())
        .merge(asset::router())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ));

    // Capture uploads can legitimately run for minutes; keep them outside the
    // default deadline and give the group its own longer one.
    let long_running = Router::new()
        .nest("/api", api::long_running())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            CAPTURE_TIMEOUT,
        ));

    let request_id = request_id_header();
    let app = normal
        .merge(long_running)
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(from_fn_with_state(
            Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
            shed_requests,
        ))
        .layer(DefaultBodyLimit::max(DEFAULT_REQUEST_BYTES))
        .with_state(state);

    probes.merge(app).layer(from_fn(security::headers))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    use super::*;
    use crate::{
        config::{DatabaseSettings, MailSettings},
        db,
        infra::{BlobFiles, Mailer, downloader},
        util::time::now_ms,
    };

    async fn test_state() -> AppState {
        let settings = DatabaseSettings {
            url: "sqlite::memory:".into(),
            ..DatabaseSettings::default()
        };
        let db = db::Database::open(&settings)
            .await
            .expect("in-memory sqlite");
        db::migrate(db.writer()).await.expect("migrations");

        let blob_dir = std::env::temp_dir().join(format!(
            "web-archiver-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let blobs = BlobFiles::open(blob_dir, 1024).await.expect("blob store");
        let fetch_client = downloader::client_builder().build().expect("http client");
        let mailer = Arc::new(Mailer::from_settings(&MailSettings::default()).expect("mailer"));
        let config = Arc::new(RuntimeConfig {
            public_url: "http://localhost".to_owned(),
            secure_cookie: false,
            trusted_proxies: Vec::new(),
        });

        AppState {
            db,
            blobs,
            fetch_client,
            mailer,
            config,
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
