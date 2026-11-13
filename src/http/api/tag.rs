use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, patch, post},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    auth::AuthUser,
    db::helper::FieldPatch,
    error::Result,
    http::extract::body::{AppJson, AppPath},
    infra::TagId,
    service,
};

/// GET /api/tag
async fn list_tags(
    State(ctx): State<AppState>,
    user: AuthUser,
) -> Result<AppJson<serde_json::Value>> {
    let tags = service::tag::list(&ctx, &user).await?;
    Ok(AppJson(serde_json::json!({ "tags": tags })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateTagRequest {
    pub name: String,
    pub color: Option<String>,
}

/// POST /api/tag
async fn create_tag(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<CreateTagRequest>,
) -> Result<(StatusCode, AppJson<crate::db::tag::Tag>)> {
    let tag = service::tag::create(&ctx, &user, &req.name, req.color.as_deref()).await?;
    Ok((StatusCode::CREATED, AppJson(tag)))
}

/// PATCH semantics: `color: null` clears, an absent field leaves it unchanged.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchTagRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub color: FieldPatch<String>,
}

async fn patch_tag(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<TagId>,
    AppJson(req): AppJson<PatchTagRequest>,
) -> Result<AppJson<crate::db::tag::Tag>> {
    let tag = service::tag::patch(&ctx, &user, id, req.name.as_deref(), req.color).await?;
    Ok(AppJson(tag))
}

/// DELETE /api/tag/{id}
async fn delete_tag(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<TagId>,
) -> Result<StatusCode> {
    service::tag::delete(&ctx, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeTagRequest {
    /// The surviving tag. Serialized as `"into"` to keep the existing wire format.
    #[serde(rename = "into")]
    pub target: TagId,
}

async fn merge_tag(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(source): AppPath<TagId>,
    AppJson(req): AppJson<MergeTagRequest>,
) -> Result<AppJson<crate::db::tag::Tag>> {
    let tag = service::tag::merge(&ctx, &user, source, req.target).await?;
    Ok(AppJson(tag))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/tag", get(list_tags).post(create_tag))
        .route("/tag/{id}", patch(patch_tag).delete(delete_tag))
        .route("/tag/{id}/merge", post(merge_tag))
}
