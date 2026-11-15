//! Pending-queue API. All operations are owner-scoped by the service.

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get as get_route, post},
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    db::{
        cursor::{Cursor, CursorPage},
        pending::{PendingId, PendingResponse, PendingStatus},
    },
    error::Result,
    identity::AuthUser,
    service,
    transport::extract::body::{AppJson, AppPath, AppQuery},
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

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<ListPendingQuery>,
) -> Result<AppJson<CursorPage<PendingResponse>>> {
    Ok(AppJson(
        service::pending::list(&state.db, &user, query.status, query.cursor, query.limit).await?,
    ))
}

async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<CreatePendingRequest>,
) -> Result<(StatusCode, AppJson<PendingResponse>)> {
    let item = service::pending::create(&state.db, &user, &req.url, &req.tags).await?;
    Ok((StatusCode::CREATED, AppJson(item)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BulkCreateRequest {
    items: Vec<CreatePendingRequest>,
}

async fn bulk_create(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<BulkCreateRequest>,
) -> Result<AppJson<service::pending::BulkCreateReport>> {
    let items: Vec<(String, Vec<String>)> = req
        .items
        .into_iter()
        .map(|item| (item.url, item.tags))
        .collect();
    Ok(AppJson(
        service::pending::bulk_create(&state.db, &user, &items).await?,
    ))
}

async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
) -> Result<AppJson<PendingResponse>> {
    Ok(AppJson(service::pending::get(&state.db, &user, id).await?))
}

async fn patch(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
    AppJson(req): AppJson<PatchPendingRequest>,
) -> Result<AppJson<PendingResponse>> {
    Ok(AppJson(
        service::pending::patch(
            &state.db,
            &user,
            id,
            req.status,
            req.url.as_deref(),
            req.tags.as_deref(),
        )
        .await?,
    ))
}

async fn retry(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
) -> Result<AppJson<PendingResponse>> {
    Ok(AppJson(
        service::pending::retry(&state.db, &user, id).await?,
    ))
}

/// Atomically take the oldest queued item. 204 when the queue is empty.
async fn claim(State(state): State<AppState>, user: AuthUser) -> Result<Response> {
    Ok(service::pending::claim(&state.db, &user)
        .await?
        .map_or_else(
            || StatusCode::NO_CONTENT.into_response(),
            |item| AppJson(item).into_response(),
        ))
}

async fn fail(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
    AppJson(req): AppJson<FailPendingRequest>,
) -> Result<AppJson<PendingResponse>> {
    Ok(AppJson(
        service::pending::fail(&state.db, &user, id, &req.error).await?,
    ))
}

async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
) -> Result<StatusCode> {
    service::pending::delete(&state.db, &user, id).await?;
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

async fn clear(
    State(state): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<ClearPendingQuery>,
) -> Result<AppJson<ClearPendingResponse>> {
    let deleted = service::pending::clear(&state.db, &user, query.status).await?;
    Ok(AppJson(ClearPendingResponse { deleted }))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/pending", get_route(list).post(create).delete(clear))
        .route("/pending/bulk", post(bulk_create))
        .route("/pending/claim", post(claim))
        .route("/pending/{id}", get_route(get).patch(patch).delete(delete))
        .route("/pending/{id}/retry", post(retry))
        .route("/pending/{id}/fail", post(fail))
}
