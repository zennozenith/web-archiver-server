//! ── Health & maintenance ────────────────────────────────────────────────────
//!   GET    /api/health                   JSON { status: "ok" }
//!   GET    /api/stats                    totals, dedup savings, queue depth
//!   POST   /api/maintenance/gc           delete unreferenced content and orphan files
//!
//! ── Collections (nested folders that group pages) ───────────────────────────
//!   GET    /api/collection               all collections, with direct link counts
//!   POST   /api/collection               JSON { name, description?, `parentId`?, color? }
//!   GET    /api/collection/:id           one collection
//!   POST   /api/collection/:id           update (same body as create; name required)
//!   PUT    /api/collection/:id           same as POST
//!   DELETE /api/collection/:id           removes the collection, its pages and captures;
//!                                        child collections are detached (204)
//!   PUT    /api/collection/:id/sharing   JSON { public? } non-null value → public
//!   GET    /api/public/collection/:id    one collection, only when `public` is true
//!
//! ── Pending queue ───────────────────────────────────────────────────────────
//!   GET    /api/pending                  ?`status=queued|in_progress|failed` &limit &cursor
//!   POST   /api/pending                  JSON { url, tags?: "a,b" | [names] }
//!   DELETE /api/pending                  ?status=queued|failed → { deleted }
//!   POST   /api/pending/bulk             JSON { items: [{ url, tags? }] } (1..=500)
//!   GET    /api/pending/:id
//!   PATCH  /api/pending/:id              JSON { tags?, status? }
//!   DELETE /api/pending/:id
//!   POST   /api/pending/:id/retry        reset failed → queued
//!   POST   /api/pending/claim            worker atomically takes next queued item → `in_progress`
//!                                        (204 when the queue is empty)
//!   POST   /api/pending/:id/fail         JSON { error } increments attempts, sets failed
//!
//! ── Pages (a URL and all its versions) ──────────────────────────────────────
//!   GET    /api/page                     ?text &field=name|url|text|tags
//!                                        &sort=newest|oldest|name
//!                                        &pinnedOnly &collectionId &tagId &limit &cursor
//!   POST   /api/page                     multipart/form-data: url, `displayTitle`,
//!                                        description?, `collectionId`?, pinned?,
//!                                        tags? (comma-separated), thumbnail? → page item
//!   GET    /api/page/lookup              ?url=...  (is this URL archived? returns page + latest capture)
//!   GET    /api/page/:id                 url, tags, capture count, latest capture
//!   PATCH  /api/page/:id                 JSON { `displayTitle`?, description?, `collectionId`?,
//!                                        pinned?, tags? } (absent = unchanged; 204)
//!   DELETE /api/page/:id                 removes page, captures, tag links (orphan content GC'd)
//!   GET    /api/page/:id/thumbnail       stored thumbnail blob
//!   GET    /api/page/:id/capture         ?limit &cursor  version history, newest first
//!                                        → { items: [...], nextCursor }
//!   PUT    /api/page/:id/tag             JSON { tags: [{ name, color? }] } replace the full set
//!   POST   /api/page/:id/tag             JSON { name } add one (get-or-create)
//!   DELETE /api/page/:id/tag/`:tag_id`
//!
//! ── Captures (a single version) ─────────────────────────────────────────────
//!   POST   /api/page/:id/capture         multipart/form-data: html, screenshot?,
//!                                        metadata? (JSON), title?, `pendingId`?,
//!                                        saveAssets? (default false; fetch and store the
//!                                        document's stylesheets/scripts/images/... as
//!                                        content blobs)
//!                                        → 201 { `capture_id`, `page_id`, version,
//!                                                deduplicated: {html, originalHtml, screenshot},
//!                                                assets: {saved, deduplicated, failed, skipped} }
//!   GET    /api/capture/:id              metadata, headers, sizes, title, version number
//!   DELETE /api/capture/:id
//!   GET    /api/capture/:id/html         archived document with resource URLs rewritten
//!                                        to /api/content/:hash
//!   GET    /api/capture/:id/html/original  exact bytes uploaded to /api/page/:id/capture
//!   GET    /api/capture/:id/screenshot   image with the stored format's Content-Type
//!   GET    /api/capture/:id/asset        sub-resources saved with this capture
//!   GET    /api/capture/:id/asset/`:content_id`
//!   GET    /api/page/:id/latest/html     307 → latest capture's html
//!   GET    /api/page/:id/latest/html/original  307 → latest capture's original html
//!   GET    /api/page/:id/latest/screenshot      307 → latest capture's screenshot
//!   GET    /api/page/:id/latest/asset    assets of the latest capture
//!
//! ── Content (raw deduplicated blobs) ────────────────────────────────────────
//!   GET    /api/content/:hash            hex blake3; immutable
//!   HEAD   /api/content/:hash            exists? (lets clients skip uploading known files)
//!
//! ── Tags ────────────────────────────────────────────────────────────────────
//!   GET    /api/tag                      with page and pending counts
//!   POST   /api/tag                      JSON { name, color? }
//!   PATCH  /api/tag/:id                  JSON { name?, color? } (color: null clears)
//!   DELETE /api/tag/:id
//!   POST   /api/tag/:id/merge            JSON { into: `tag_id` }

// mod archive;
// mod asset;
// mod capture;
// mod collection;
// mod content;
mod misc;
// mod page;
// mod pending;
// mod tag;

pub(crate) mod admin;
pub(crate) mod auth;
pub(crate) mod me;

use axum::{
    Router,
    extract::{Request, State},
    middleware::Next,
    response::Response,
    routing::get,
};
use http::header::AUTHORIZATION;

use crate::prelude::*;

// #[expect(
//     dead_code,
//     reason = "bearer-auth middleware is kept for deployments that enable it"
// )]
// async fn require_auth(State(state): State<AppState>, req: Request, next: Next) -> Result<Response> {
//     let ok = req
//         .headers()
//         .get(AUTHORIZATION)
//         .and_then(|v| v.to_str().ok())
//         .and_then(|v| v.strip_prefix("Bearer "))
//         // blake3::Hash equality is constant-time
//         .is_some_and(|t| blake3::hash(t.as_bytes()) == blake3::hash(state.token.as_bytes()));
//     if !ok {
//         return Err(AppError::Unauthorized);
//     }
//     Ok(next.run(req).await)
// }

pub(crate) fn router() -> Router<AppState> {
    let api = Router::<AppState>::new()
        .merge(auth::routes())
        .merge(me::routes())
        .merge(admin::routes());
    // .merge(tag::routes())
    // .merge(collection::routes())
    // .merge(pending::routes())
    // .merge(page::routes())
    // .merge(capture::routes())
    // .merge(content::routes())
    // .merge(misc::routes());
    // .layer(from_fn_with_state (state.clone(), require_auth));

    Router::new().route("/health", get(misc::health)).merge(api)
}
