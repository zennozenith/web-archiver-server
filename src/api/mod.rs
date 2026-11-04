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

mod archive;
mod asset;
mod capture;
mod collection;
mod content;
mod misc;
mod page;
mod pending;
mod tag;

pub(crate) use page::parse_multipart;

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request, State},
    middleware::Next,
    response::Response,
    routing::{delete, get, post, put},
};
use http::header::AUTHORIZATION;

use crate::{
    AppState,
    api::{
        capture::{
            get_capture_html, get_capture_original_html, get_capture_screenshot, get_thumbnail,
            redirect_latest_html, redirect_latest_original_html, redirect_latest_screenshot,
        },
        collection::{
            create_collection, delete_collection, get_collection, list_collection,
            public_collection, update_collection, update_sharing,
        },
        page::{create_page, list_page_captures, patch_page},
        pending::list_pending,
    },
    error::{AppError, Result},
};
use archive::archive;
use asset::{capture_asset, capture_assets, latest_assets};
use capture::{delete_capture, get_capture};
use content::{get_content, head_content};
use misc::{gc_content, health, stats};
use page::{
    add_page_tag, delete_page, get_page, list_pages, lookup_page, put_page_tags, remove_page_tag,
};
use pending::{
    bulk_create_pending, claim_pending, clear_pending, create_pending, delete_pending,
    fail_pending, get_pending, patch_pending, retry_pending,
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
    let api = Router::<AppState>::new()
        .merge(tag_routes())
        .merge(collection_routes())
        .merge(pending_routes())
        .merge(page_routes())
        .merge(capture_routes())
        .merge(content_routes())
        .merge(misc_routes());
    // .layer(from_fn_with_state (state.clone(), require_auth));

    Router::new()
        .route("/health", get(health))
        .merge(api)
        .with_state(state)
}

fn tag_routes() -> Router<AppState> {
    Router::new()
        .route("/tag", get(list_tags).post(create_tag))
        .route(
            "/tag/{id}",
            axum::routing::patch(patch_tag).delete(delete_tag),
        )
        .route("/tag/{id}/merge", post(merge_tag))
}

fn collection_routes() -> Router<AppState> {
    Router::new()
        .route("/collection", get(list_collection).post(create_collection))
        .route(
            "/collection/{id}",
            get(get_collection)
                .post(update_collection)
                .put(update_collection)
                .delete(delete_collection),
        )
        .route("/collection/{id}/sharing", put(update_sharing))
        .route("/public/collection/{id}", get(public_collection))
}

fn pending_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/pending",
            get(list_pending).post(create_pending).delete(clear_pending),
        )
        .route("/pending/bulk", post(bulk_create_pending))
        .route("/pending/claim", post(claim_pending))
        .route(
            "/pending/{id}",
            get(get_pending).patch(patch_pending).delete(delete_pending),
        )
        .route("/pending/{id}/retry", post(retry_pending))
        .route("/pending/{id}/fail", post(fail_pending))
}

fn page_routes() -> Router<AppState> {
    Router::new()
        .route("/page", get(list_pages).post(create_page))
        .route("/page/lookup", get(lookup_page))
        .route(
            "/page/{id}",
            get(get_page).patch(patch_page).delete(delete_page),
        )
        .route("/page/{id}/thumbnail", get(get_thumbnail)) // TODO:
        .route(
            "/page/{id}/capture",
            get(list_page_captures)
                .post(archive)
                .layer(DefaultBodyLimit::max(ARCHIVE_BODY_LIMIT)),
        )
        .route("/page/{id}/tag", post(add_page_tag).put(put_page_tags))
        .route("/page/{id}/tag/{tag_id}", delete(remove_page_tag))
        .route("/page/{id}/latest/html", get(redirect_latest_html))
        .route(
            "/page/{id}/latest/html/original",
            get(redirect_latest_original_html),
        )
        .route(
            "/page/{id}/latest/screenshot",
            get(redirect_latest_screenshot),
        )
}

fn capture_routes() -> Router<AppState> {
    Router::new()
        .route("/capture/{id}", get(get_capture).delete(delete_capture))
        .route("/capture/{id}/html", get(get_capture_html))
        .route(
            "/capture/{id}/html/original",
            get(get_capture_original_html),
        )
        .route("/capture/{id}/screenshot", get(get_capture_screenshot))
        .route("/capture/{id}/asset", get(capture_assets))
        .route("/capture/{id}/asset/{content_id}", get(capture_asset))
        .route("/page/{id}/latest/asset", get(latest_assets))
}

fn content_routes() -> Router<AppState> {
    // explicit HEAD handler avoids loading the blob
    Router::new().route("/content/{hash}", get(get_content).head(head_content))
}

fn misc_routes() -> Router<AppState> {
    Router::new()
        .route("/stats", get(stats))
        .route("/maintenance/gc", post(gc_content))
}
