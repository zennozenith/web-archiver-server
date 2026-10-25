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

mod archive;
mod captures;
mod health;
mod pages;
mod pending;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
};

use crate::AppState;
use archive::*;
use captures::*;
use pages::*;
use pending::*;

const ARCHIVE_BODY_LIMIT: usize = 64 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        // health
        .route("/health", get(health::health))
        // archive
        .route(
            "/archive",
            post(archive).layer(DefaultBodyLimit::max(ARCHIVE_BODY_LIMIT)),
        )
        // pending
        .route(
            "/pending",
            get(list_pending_handler)
                .post(create_pending)
                .delete(clear_pending),
        )
        .route("/pending/bulk", post(bulk_create_pending))
        .route("/pending/claim", post(claim_pending))
        .route(
            "/pending/{id}",
            get(get_pending).patch(patch_pending).delete(delete_pending),
        )
        .route("/pending/{id}/retry", post(retry_pending))
        .route("/pending/{id}/fail", post(fail_pending))
        // pages
        .route("/api/pages", get(list_pages))
        .route("/api/pages/lookup", get(lookup_page))
        .route("/api/pages/{id}", get(get_page).delete(delete_page))
        .route("/api/pages/{id}/captures", get(list_captures))
        .route(
            "/api/pages/{id}/tags",
            post(add_page_tag).put(put_page_tags),
        )
        .route("/api/pages/{id}/tags/{tag_id}", delete(remove_page_tag))
        .route("/api/pages/{id}/latest/html", get(latest_html))
        .route("/api/pages/{id}/latest/screenshot", get(latest_screenshot))
        // captures
        .route("/captures/{id}", get(get_capture).delete(delete_capture))
        .route("/captures/{id}/html", get(capture_html))
        .route("/captures/{id}/screenshot", get(capture_screenshot))
}
