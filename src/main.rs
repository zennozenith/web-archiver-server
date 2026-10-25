use axum::{Router, extract::FromRef};
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

use crate::config::Settings;

mod api;
mod assets;
mod compress;
mod config;
mod db;
mod downloader;
mod dto;
mod error;
mod extract;
mod extractor;
mod htmx;
mod models;
mod scrapper;
mod serve;
mod tag;
mod template;
mod time;
mod url_canon;
mod web;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
}

impl FromRef<AppState> for SqlitePool {
    fn from_ref(app_state: &AppState) -> SqlitePool {
        app_state.db.clone()
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let settings = Settings::load();

    let db = db::create_pool(&settings.database)
        .await
        .expect("Unable to create database");

    let state = AppState { db };

    let app = app(state);

    dbg!(&settings);

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

    let _ = axum::serve(listener, app).await;
}

fn app(state: AppState) -> Router {
    Router::new()
        .nest("/api", api::routes())
        .merge(web::routes())
        .with_state(state)
        .merge(assets::serve_dir())
        .layer(axum::middleware::from_fn(web::negotiate_errors))
        .layer(TraceLayer::new_for_http())
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use axum::{
//         body::Body,
//         http::{self, Request, StatusCode},
//     };
//     use http_body_util::BodyExt;
//     use tower::ServiceExt; // for `call`, `oneshot`, and `ready` // for `collect`

//     #[tokio::test]
//     async fn test_get() {
//         let app = app();

//         let response = app
//             .oneshot(Request::get("/").body(Body::empty()).unwrap())
//             .await
//             .unwrap();

//         assert_eq!(response.status(), StatusCode::OK);

//         let body = response.into_body().collect().await.unwrap().to_bytes();
//         let body = std::str::from_utf8(&body).unwrap();

//         assert!(body.contains(r#"<input type="submit" value="Subscribe!">"#));
//     }

//     #[tokio::test]
//     async fn test_post() {
//         let app = app();

//         let response = app
//             .oneshot(
//                 Request::post("/")
//                     .header(
//                         http::header::CONTENT_TYPE,
//                         mime::APPLICATION_WWW_FORM_URLENCODED.as_ref(),
//                     )
//                     .body(Body::from("name=foo&email=bar@axum"))
//                     .unwrap(),
//             )
//             .await
//             .unwrap();

//         assert_eq!(response.status(), StatusCode::OK);

//         let body = response.into_body().collect().await.unwrap().to_bytes();
//         let body = std::str::from_utf8(&body).unwrap();

//         assert_eq!(body, "email='bar@axum'\nname='foo'\n");
//     }
// }
