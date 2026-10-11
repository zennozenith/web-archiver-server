use askama::Template;
use axum::{Router, response::IntoResponse, routing::get};
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

use crate::error::AppError;

mod assets;
mod core;
mod error;
mod scrapper;

use core::RenderHtml;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    title: &'a str,
}

#[derive(Template)]
#[template(path = "vite-counter.html")]
struct ViteCounter<'a> {
    title: &'a str,
}

#[derive(Template)]
#[template(path = "colonist.html")]
struct Colonist<'a> {
    title: &'a str,
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

    let app = app();

    // let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    let _ = axum::serve(listener, app).await;
}

fn app() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/counter", get(vite_counter))
        .route("/colonist", get(colonist))
        .route("/anyhow", get(try_thing))
        // .route("/form1", get(show_form).post(accept_form))
        .merge(assets::serve_dir())
}

async fn index() -> Result<impl IntoResponse, AppError> {
    IndexTemplate {
        title: "Rust + Axum + Vite + Alpine",
    }
    .render_html()
}

async fn vite_counter() -> Result<impl IntoResponse, AppError> {
    ViteCounter {
        title: "Rust + Axum + Vite + Alpine",
    }
    .render_html()
}

async fn colonist() -> Result<impl IntoResponse, AppError> {
    Colonist { title: "Colonist" }.render_html()
}

async fn try_thing() -> Result<(), AppError> {
    Err(anyhow::anyhow!("it failed!").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{self, Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt; // for `call`, `oneshot`, and `ready` // for `collect`

    #[tokio::test]
    async fn test_get() {
        let app = app();

        let response = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = std::str::from_utf8(&body).unwrap();

        assert!(body.contains(r#"<input type="submit" value="Subscribe!">"#));
    }

    #[tokio::test]
    async fn test_post() {
        let app = app();

        let response = app
            .oneshot(
                Request::post("/")
                    .header(
                        http::header::CONTENT_TYPE,
                        mime::APPLICATION_WWW_FORM_URLENCODED.as_ref(),
                    )
                    .body(Body::from("name=foo&email=bar@axum"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = std::str::from_utf8(&body).unwrap();

        assert_eq!(body, "email='bar@axum'\nname='foo'\n");
    }
}
