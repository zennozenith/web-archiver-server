//! ── Archive (create a new version) ──────────────────────────────────────────
//!   POST   /api/archive                  multipart/form-data: html, screenshot, screenshot-format
//!                                        ("png"|"jpeg"), metadata (JSON), + optional url, title,
//!                                        tags (JSON array of names), pending_id
//!                                        → 201 { capture_id, page_id, version, deduplicated: {html, screenshot} }
//!
//! ── Pending queue ───────────────────────────────────────────────────────────
//!   GET    /api/pending                  ?status=queued|in_progress|failed &limit &cursor
//!   POST   /api/pending                  JSON { url, title?, tags?: [names] }
//!   POST   /api/pending/bulk             JSON { items: [{ url, title?, tags? }] }
//!   GET    /api/pending/:id
//!   PATCH  /api/pending/:id              JSON { title?, tags?, status? }
//!   DELETE /api/pending/:id
//!   POST   /api/pending/:id/retry        reset failed → queued
//!   POST   /api/pending/claim            worker atomically takes next queued item → in_progress
//!   POST   /api/pending/:id/fail         JSON { error } increments attempts, sets failed
//!
//! ── Pages (a URL and all its versions) ──────────────────────────────────────
//!   GET    /api/pages                    ?tag=a&tag=b &match=all|any &host &q &sort=recent|title &limit &cursor
//!   GET    /api/pages/lookup             ?url=...  (is this URL archived? returns page + latest capture)
//!   GET    /api/pages/:id                url, tags, capture count, latest capture
//!   DELETE /api/pages/:id                removes page, captures, tag links (orphan content GC'd)
//!   GET    /api/pages/:id/captures       version history, newest first
//!   PUT    /api/pages/:id/tags           JSON { tags: [names] } replace the full set
//!   POST   /api/pages/:id/tags           JSON { name } add one (get-or-create)
//!   DELETE /api/pages/:id/tags/:tag_id
//!
//! ── Captures (a single version) ─────────────────────────────────────────────
//!   GET    /api/captures/:id             metadata, headers, sizes, title, version number
//!   DELETE /api/captures/:id
//!   GET    /api/captures/:id/html        archived document (see serving notes)
//!   GET    /api/captures/:id/screenshot  image with the stored format's Content-Type
//!   GET    /api/pages/:id/latest/html    302 → latest capture's html
//!   GET    /api/pages/:id/latest/screenshot
//!
//! ── Content (raw deduplicated blobs) ────────────────────────────────────────
//!   GET    /api/content/:hash            hex blake3; immutable
//!   HEAD   /api/content/:hash            exists? (lets clients skip uploading known files)
//!
//! ── Tags ────────────────────────────────────────────────────────────────────
//!   GET    /api/tags                     with page counts
//!   POST   /api/tags                     JSON { name, color? }
//!   PATCH  /api/tags/:id                 JSON { name?, color? }
//!   DELETE /api/tags/:id
//!   POST   /api/tags/:id/merge           JSON { into: tag_id }
//!
//! ── Misc ────────────────────────────────────────────────────────────────────
//!   GET    /api/search                   ?q= (FTS5, later)
//!   GET    /api/stats                    totals, dedup savings, queue depth
//!   GET    /api/health

use askama::Template;
use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Serialize;

pub mod api;

use crate::{
    AppState,
    core::RenderHtml as _,
    error::{AppError, Result},
};
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

pub async fn index() -> Result<impl IntoResponse, AppError> {
    IndexTemplate {
        title: "Rust + Axum + Vite + Alpine",
    }
    .render_html()
}

pub async fn vite_counter() -> Result<impl IntoResponse, AppError> {
    ViteCounter {
        title: "Rust + Axum + Vite + Alpine",
    }
    .render_html()
}

#[derive(Debug, Serialize)]
struct PendingUrl {
    id: String,
    url: String,
    title: String,
}

async fn get_pending() -> Result<Json<Vec<PendingUrl>>, AppError> {
    Ok(Json(vec![
        PendingUrl {
            id: "1".into(),
            url: "https://a.example.com".into(),
            title: "A example".into(),
        },
        PendingUrl {
            id: "2".into(),
            url: "https://b.example.com".into(),
            title: "B example".into(),
        },
        PendingUrl {
            id: "3".into(),
            url: "https://c.example.com".into(),
            title: "C example".into(),
        },
        PendingUrl {
            id: "4".into(),
            url: "https://docs.rs/axum/latest/axum/struct.Json.html".into(),
            title: "Axum Json".into(),
        },
    ]))
}

pub fn api() -> Router<AppState> {
    Router::new()
        .route("/health", get(api::health))
        .route("/pending", post(api::pending::create_pending))
        // .route("/archive", post(api::archive))
        // .route("/pending", get(get_pending))
        // .route("/check", get(get_pending))
        .layer(DefaultBodyLimit::max(1024 * 4))
}
