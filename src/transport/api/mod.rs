//! ── Health & maintenance ────────────────────────────────────────────────────
//!   GET    /api/health                   JSON { status: "ok" }
//!   GET    /api/stats                    totals, dedup savings, queue depth (admin)
//!   POST   /api/maintenance/gc           delete unreferenced content (admin)
//!
//! ── Collections (flat, per-owner folders that group pages) ──────────────────
//!   GET    /api/collection               all collections, with direct link counts
//!   POST   /api/collection               JSON { name, emoji?, description?, color? }
//!   GET    /api/collection/:id           one collection
//!   POST   /api/collection/:id           update (same body as create; name required)
//!   PUT    /api/collection/:id           same as POST
//!   DELETE /api/collection/:id           removes the collection; pages become unorganized
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
//!   POST   /api/pending/claim            worker atomically takes next queued item
//!   POST   /api/pending/:id/fail         JSON { error } increments attempts, sets failed
//!
//! ── Pages (a URL and all its versions) ──────────────────────────────────────
//!   GET    /api/page                     ?text &field=name|url|text|tags
//!                                        &sort=newest|oldest|name
//!                                        &pinnedOnly &collectionId &unorganizedOnly
//!                                        &tagId &limit &cursor
//!   POST   /api/page                     multipart/form-data: url, `displayTitle`,
//!                                        description?, `collectionId`?, pinned?,
//!                                        tags? (comma-separated), thumbnail?
//!   GET    /api/page/lookup              ?url=...
//!   GET    /api/page/:id                 url, tags, capture count, latest capture
//!   PATCH  /api/page/:id                 JSON { `displayTitle`?, description?, `collectionId`?,
//!                                        pinned?, tags? } (absent = unchanged; 204)
//!   DELETE /api/page/:id                 removes page, captures, tag links
//!   GET    /api/page/:id/thumbnail       stored thumbnail blob
//!   GET    /api/page/:id/capture         ?limit &cursor  version history
//!   PUT    /api/page/:id/tag             JSON { tags: [{ name }] } replace the full set
//!   POST   /api/page/:id/tag             JSON { name } add one (get-or-create)
//!   DELETE /api/page/:id/tag/`:tag_id`
//!
//! ── Captures (a single version) ─────────────────────────────────────────────
//!   POST   /api/page/:id/capture         multipart/form-data: html, screenshot?,
//!                                        metadata? (JSON), title?, `pendingId`?,
//!                                        saveAssets?
//!   GET    /api/capture/:id              metadata, headers, sizes, title, version number
//!   DELETE /api/capture/:id
//!   GET    /api/capture/:id/html         archived document with rewritten resource URLs
//!   GET    /api/capture/:id/html/original  exact bytes uploaded
//!   GET    /api/capture/:id/screenshot   image with the stored format's Content-Type
//!   GET    /api/capture/:id/asset        sub-resources saved with this capture
//!   GET    /api/capture/:id/asset/`:content_id`
//!   GET    /api/page/:id/latest/html     307 → latest capture's html
//!   GET    /api/page/:id/latest/html/original
//!   GET    /api/page/:id/latest/screenshot
//!   GET    /api/page/:id/latest/asset    assets of the latest capture
//!
//! ── Content (raw deduplicated blobs) ────────────────────────────────────────
//!   GET    /api/content/:hash            hex blake3; immutable, owner-visible only
//!   HEAD   /api/content/:hash            exists?
//!
//! ── Tags ────────────────────────────────────────────────────────────────────
//!   GET    /api/tag                      with page and pending counts
//!   POST   /api/tag                      JSON { name, color? }
//!   PATCH  /api/tag/:id                  JSON { name?, color? } (color: null clears)
//!   DELETE /api/tag/:id
//!   POST   /api/tag/:id/merge            JSON { into: `tag_id` }

mod archive;
mod asset;
mod capture;
mod collection;
mod content;
mod page;
mod pending;
mod tag;

pub(crate) mod admin;
pub(crate) mod auth;
pub(crate) mod me;
mod misc;

use axum::{Router, routing::get};

use crate::app::AppState;

pub(crate) fn router() -> Router<AppState> {
    let api = Router::<AppState>::new()
        .merge(auth::routes())
        .merge(me::routes())
        .merge(admin::routes())
        .merge(tag::routes())
        .merge(collection::routes())
        .merge(pending::routes())
        .merge(page::routes())
        .merge(capture::routes())
        .merge(content::routes())
        .merge(misc::routes());

    Router::new().route("/health", get(misc::health)).merge(api)
}

/// Multipart uploads that get `UPLOAD_TIMEOUT` instead of the default deadline.
///
/// Mounted by [`crate::app::app`] next to (not inside) the default group; the
/// paths overlap with [`router`] but the methods do not.
pub(crate) fn upload_routes() -> Router<AppState> {
    Router::new().merge(page::upload_route())
}

/// Routes that intentionally exceed the default request deadline.
///
/// They are mounted outside the global timeout layer by [`crate::app::app`].
pub(crate) fn long_running() -> Router<AppState> {
    Router::new().merge(page::capture_route())
}
