use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, patch as patch_route, post},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    db::patch::FieldPatch,
    domain::TagId,
    error::Result,
    identity::AuthUser,
    service,
    transport::extract::body::{AppJson, AppPath},
};

/// GET /api/tag
async fn list(State(state): State<AppState>, user: AuthUser) -> Result<AppJson<serde_json::Value>> {
    let tags = service::tag::list(&state.db, &user).await?;
    Ok(AppJson(serde_json::json!({ "tags": tags })))
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateTagRequest {
    pub name: String,
    pub color: Option<String>,
}

/// POST /api/tag
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<CreateTagRequest>,
) -> Result<(StatusCode, AppJson<crate::db::tag::Tag>)> {
    let tag = service::tag::create(&state.db, &user, &req.name, req.color.as_deref()).await?;
    Ok((StatusCode::CREATED, AppJson(tag)))
}

/// PATCH semantics: `color: null` clears, an absent field leaves it unchanged.
#[derive(Debug, Deserialize)]
struct PatchTagRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub color: FieldPatch<String>,
}

async fn patch(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<TagId>,
    AppJson(req): AppJson<PatchTagRequest>,
) -> Result<AppJson<crate::db::tag::Tag>> {
    let tag = service::tag::patch(&state.db, &user, id, req.name.as_deref(), req.color).await?;
    Ok(AppJson(tag))
}

/// DELETE /api/tag/{id}
async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<TagId>,
) -> Result<StatusCode> {
    service::tag::delete(&state.db, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct MergeTagRequest {
    /// The surviving tag. Serialized as `"into"` to keep the existing wire format.
    #[serde(rename = "into")]
    pub target: TagId,
}

async fn merge(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(source): AppPath<TagId>,
    AppJson(req): AppJson<MergeTagRequest>,
) -> Result<AppJson<crate::db::tag::Tag>> {
    let tag = service::tag::merge(&state.db, &user, source, req.target).await?;
    Ok(AppJson(tag))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/tag", get(list).post(create))
        .route("/tag/{id}", patch_route(patch).delete(delete))
        .route("/tag/{id}/merge", post(merge))
}
