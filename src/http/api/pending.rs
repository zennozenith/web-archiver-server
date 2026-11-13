//! Pending-queue API. All operations are owner-scoped by the service.

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    auth::AuthUser,
    db::{
        helper::{Cursor, CursorPage},
        pending::{PendingId, PendingItem, PendingStatus},
    },
    error::Result,
    http::extract::body::{AppJson, AppPath, AppQuery},
    service,
    util::{
        limit::Limit,
        serde::{comma_separated_or_seq, opt_comma_separated_or_seq},
    },
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePendingRequest {
    url: String,
    #[serde(default, deserialize_with = "comma_separated_or_seq")]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchPendingRequest {
    #[serde(default, deserialize_with = "opt_comma_separated_or_seq")]
    tags: Option<Vec<String>>,
    status: Option<PendingStatus>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FailPendingRequest {
    error: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListPendingQuery {
    status: Option<PendingStatus>,
    limit: Option<Limit>,
    cursor: Option<Cursor>,
}

async fn list_pending(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<ListPendingQuery>,
) -> Result<AppJson<CursorPage<PendingItem>>> {
    Ok(AppJson(
        service::pending::list(&ctx, &user, query.status, query.cursor, query.limit).await?,
    ))
}

async fn create_pending(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<CreatePendingRequest>,
) -> Result<(StatusCode, AppJson<PendingItem>)> {
    let item = service::pending::create(&ctx, &user, &req.url, &req.tags).await?;
    Ok((StatusCode::CREATED, AppJson(item)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BulkCreateRequest {
    items: Vec<CreatePendingRequest>,
}

async fn bulk_create_pending(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<BulkCreateRequest>,
) -> Result<AppJson<service::pending::BulkCreateReport>> {
    let items: Vec<(String, Vec<String>)> = req
        .items
        .into_iter()
        .map(|item| (item.url, item.tags))
        .collect();
    Ok(AppJson(
        service::pending::bulk_create(&ctx, &user, &items).await?,
    ))
}

async fn get_pending(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
) -> Result<AppJson<PendingItem>> {
    Ok(AppJson(service::pending::get(&ctx, &user, id).await?))
}

async fn patch_pending(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
    AppJson(req): AppJson<PatchPendingRequest>,
) -> Result<AppJson<PendingItem>> {
    Ok(AppJson(
        service::pending::patch(
            &ctx,
            &user,
            id,
            req.status,
            req.url.as_deref(),
            req.tags.as_deref(),
        )
        .await?,
    ))
}

async fn retry_pending(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
) -> Result<AppJson<PendingItem>> {
    Ok(AppJson(service::pending::retry(&ctx, &user, id).await?))
}

/// Atomically take the oldest queued item. 204 when the queue is empty.
async fn claim_pending(State(ctx): State<AppState>, user: AuthUser) -> Result<Response> {
    Ok(service::pending::claim(&ctx, &user).await?.map_or_else(
        || StatusCode::NO_CONTENT.into_response(),
        |item| AppJson(item).into_response(),
    ))
}

async fn fail_pending(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
    AppJson(req): AppJson<FailPendingRequest>,
) -> Result<AppJson<PendingItem>> {
    Ok(AppJson(
        service::pending::fail(&ctx, &user, id, &req.error).await?,
    ))
}

async fn delete_pending(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
) -> Result<StatusCode> {
    service::pending::delete(&ctx, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClearPendingQuery {
    status: PendingStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClearPendingResponse {
    deleted: u64,
}

async fn clear_pending(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<ClearPendingQuery>,
) -> Result<AppJson<ClearPendingResponse>> {
    let deleted = service::pending::clear(&ctx, &user, query.status).await?;
    Ok(AppJson(ClearPendingResponse { deleted }))
}

pub(super) fn routes() -> Router<AppState> {
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
