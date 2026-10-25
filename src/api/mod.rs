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
mod health;
mod pending;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};

use crate::AppState;

const ARCHIVE_BODY_LIMIT: usize = 64 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        // health
        .route("/health", get(health::health))
        // archive
        .route(
            "/archive",
            post(archive::archive).layer(DefaultBodyLimit::max(ARCHIVE_BODY_LIMIT)),
        )
        // pending
        .route(
            "/pending",
            get(pending::list_pending_handler)
                .post(pending::create_pending)
                .delete(pending::clear_pending),
        )
        .route("/pending/bulk", post(pending::bulk_create_pending))
        .route("/pending/claim", post(pending::claim_pending))
        .route(
            "/pending/{id}",
            get(pending::get_pending)
                .patch(pending::patch_pending)
                .delete(pending::delete_pending),
        )
        .route("/pending/{id}/retry", post(pending::retry_pending))
        .route("/pending/{id}/fail", post(pending::fail_pending))
}
