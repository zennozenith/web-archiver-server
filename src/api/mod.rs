//! ── Archive (create a new version) ──────────────────────────────────────────
//!   POST   /api/archive                  multipart/form-data: html, screenshot, screenshot-format
//!                                        ("png"|"jpeg"), metadata (JSON), + optional url, title,
//!                                        tags (JSON array of names), `pending_id`, save-assets
//!                                        (default true; fetch and store the document's
//!                                        stylesheets/scripts/images/... as content blobs)
//!                                        → 201 { `capture_id`, `page_id`, version,
//!                                                deduplicated: {html, originalHtml, screenshot},
//!                                                assets: {saved, deduplicated, failed, skipped} }
//!
//! ── Pending queue ───────────────────────────────────────────────────────────
//!   GET    /api/pending                  ?`status=queued|in_progress|failed` &limit &cursor
//!   POST   /api/pending                  JSON { url, title?, tags?: [names] }
//!   POST   /api/pending/bulk             JSON { items: [{ url, title?, tags? }] }
//!   GET    /api/pending/:id
//!   PATCH  /api/pending/:id              JSON { title?, tags?, status? }
//!   DELETE /api/pending/:id
//!   POST   /api/pending/:id/retry        reset failed → queued
//!   POST   /api/pending/claim            worker atomically takes next queued item → `in_progress`
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
//!   DELETE /api/pages/`:id`/tags/`:tag_id`
//!
//! ── Captures (a single version) ─────────────────────────────────────────────
//!   GET    /api/captures/:id             metadata, headers, sizes, title, version number
//!   DELETE /api/captures/:id
//!   GET    /api/captures/:id/html        archived document with resource URLs rewritten
//!                                        to /api/content/:hash
//!   GET    /api/captures/:id/html/original  exact bytes uploaded to /api/archive
//!   GET    /api/captures/:id/screenshot  image with the stored format's Content-Type
//!   GET    /api/captures/:id/assets      sub-resources saved with this capture
//!   GET    /api/captures/`:id`/assets/`:content_id`
//!   GET    /api/pages/:id/latest/html    302 → latest capture's html
//!   GET    /api/pages/:id/latest/html/original  302 → latest capture's original html
//!   GET    /api/pages/:id/latest/screenshot
//!   GET    /api/pages/:id/latest/assets
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
//!   POST   /api/tags/:id/merge           JSON { into: `tag_id` }
//!
//! ── Misc ────────────────────────────────────────────────────────────────────
//!   GET    /api/search                   ?q= (FTS5, later)
//!   GET    /api/stats                    totals, dedup savings, queue depth
//!   GET    /api/health

mod archive;
mod assets;
mod captures;
mod content;
mod misc;
mod pages;
mod pending;
mod tag;

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request, State},
    middleware::Next,
    response::Response,
    routing::{delete, get, post},
};
use http::header::AUTHORIZATION;

use crate::{
    AppState,
    error::{AppError, Result},
};
use archive::archive;
use assets::{capture_asset, capture_assets, latest_assets};
use captures::{
    capture_html, capture_original_html, capture_screenshot, delete_capture, get_capture,
    latest_html, latest_original_html, latest_screenshot,
};
use content::{get_content, head_content};
use misc::{gc_content, health, stats};
use pages::{
    add_page_tag, delete_page, get_page, list_captures, list_pages, lookup_page, put_page_tags,
    remove_page_tag,
};
use pending::{
    bulk_create_pending, claim_pending, clear_pending, create_pending, delete_pending,
    fail_pending, get_pending, list_pending_handler, patch_pending, retry_pending,
};
use tag::{create_tag, delete_tag, list_tags, merge_tag, patch_tag};

const ARCHIVE_BODY_LIMIT: usize = 64 * 1024 * 1024;

#[expect(
    dead_code,
    reason = "bearer-auth middleware is kept for deployments that enable it"
)]
async fn require_auth(State(state): State<AppState>, req: Request, next: Next) -> Result<Response> {
    let ok = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        // blake3::Hash equality is constant-time
        .is_some_and(|t| blake3::hash(t.as_bytes()) == blake3::hash(state.token.as_bytes()));
    if !ok {
        return Err(AppError::Unauthorized);
    }
    Ok(next.run(req).await)
}

pub(crate) fn router(state: AppState) -> Router {
    let api = Router::new()
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
        .route("/pages", get(list_pages))
        .route("/pages/lookup", get(lookup_page))
        .route("/pages/{id}", get(get_page).delete(delete_page))
        .route("/pages/{id}/captures", get(list_captures))
        .route("/pages/{id}/tags", post(add_page_tag).put(put_page_tags))
        .route("/pages/{id}/tags/{tag_id}", delete(remove_page_tag))
        .route("/pages/{id}/latest/html", get(latest_html))
        .route(
            "/pages/{id}/latest/html/original",
            get(latest_original_html),
        )
        .route("/pages/{id}/latest/screenshot", get(latest_screenshot))
        // captures
        .route("/captures/{id}", get(get_capture).delete(delete_capture))
        .route("/captures/{id}/html", get(capture_html))
        .route("/captures/{id}/html/original", get(capture_original_html))
        .route("/captures/{id}/screenshot", get(capture_screenshot))
        .route("/captures/{id}/assets", get(capture_assets))
        .route("/captures/{id}/assets/{content_id}", get(capture_asset))
        .route("/pages/{id}/latest/assets", get(latest_assets))
        // content (explicit HEAD handler avoids loading the blob)
        .route("/content/{hash}", get(get_content).head(head_content))
        // tags
        .route("/tags", get(list_tags).post(create_tag))
        .route(
            "/tags/{id}",
            axum::routing::patch(patch_tag).delete(delete_tag),
        )
        .route("/tags/{id}/merge", post(merge_tag))
        // misc
        .route("/stats", get(stats))
        .route("/maintenance/gc", post(gc_content));
    // .layer(from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .route("/api/health", get(health))
        .merge(api)
        .with_state(state)
}
