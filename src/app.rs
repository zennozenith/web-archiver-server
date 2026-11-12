use std::sync::Arc;

use axum::{Router, extract::FromRef};
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

use crate::{
    http::{api, asset, web},
    infra::{BlobStore, Mailer},
};

#[derive(Debug, Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub store: BlobStore,
    /// Shared client for fetching archived sub-resources.
    pub http: reqwest::Client,

    pub mailer: Arc<Mailer>,
    // pub public_url: Arc<str>,
}

impl FromRef<AppState> for SqlitePool {
    fn from_ref(app_state: &AppState) -> Self {
        app_state.db.clone()
    }
}

impl FromRef<AppState> for BlobStore {
    fn from_ref(app_state: &AppState) -> Self {
        app_state.store.clone()
    }
}

impl FromRef<AppState> for reqwest::Client {
    fn from_ref(app_state: &AppState) -> Self {
        app_state.http.clone()
    }
}

pub(super) fn app(state: AppState) -> Router {
    Router::new()
        .nest("/api", api::router())
        .merge(web::router())
        .merge(asset::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
