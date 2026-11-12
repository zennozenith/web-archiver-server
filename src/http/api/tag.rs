use std::str::FromStr;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    AppState,
    db::{
        helper::{CreateOutcome, FieldPatch},
        tag::{PatchOutcome, Tag, TagPatch, TagSummary},
    },
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath},
    tag::{TagId, TagMerge, TagName, parse_optional_color},
};

// CHECKED:
/// GET /api/tag
async fn list_tags(State(pool): State<SqlitePool>) -> Result<ApiJson<Vec<TagSummary>>> {
    let tags = TagSummary::list(&pool).await?;
    Ok(ApiJson(tags))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateTagRequest {
    pub name: String,
    pub color: Option<String>,
}

// CHECKED:
/// POST /api/tag
async fn create_tag(
    State(pool): State<SqlitePool>,
    ApiJson(req): ApiJson<CreateTagRequest>,
) -> Result<(StatusCode, ApiJson<Tag>)> {
    let name = TagName::from_str(&req.name)?;
    let color = parse_optional_color(req.color.as_deref())?;

    match Tag::create(&pool, &name, color.as_ref()).await? {
        CreateOutcome::Created(tag) => Ok((StatusCode::CREATED, ApiJson(tag))),
        CreateOutcome::Existing(tag) => Err(AppError::Duplicate {
            existing_id: tag.id.get(),
            msg: "a tag with that name already exists",
        }),
    }
}

/// PATCH semantics: `color: null` clears, an absent field leaves it unchanged.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchTagRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub color: FieldPatch<String>,
}

impl PatchTagRequest {
    /// Validate the raw payload into a [`TagPatch`]. A blank color string clears the color.
    fn into_patch(self) -> Result<TagPatch> {
        let name = self
            .name
            .as_deref()
            .map(str::parse::<TagName>)
            .transpose()?;

        let color = match self.color {
            FieldPatch::Unchanged => FieldPatch::Unchanged,
            FieldPatch::Clear => FieldPatch::Clear,
            FieldPatch::Set(raw) => {
                parse_optional_color(Some(&raw))?.map_or(FieldPatch::Clear, FieldPatch::Set)
            }
        };

        Ok(TagPatch { name, color })
    }
}

// CHECKED:
// PATCH /api/tag/{id}   (color: null clears, absent keeps)
async fn patch_tag(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<TagId>,
    ApiJson(req): ApiJson<PatchTagRequest>,
) -> Result<ApiJson<Tag>> {
    match req.into_patch()?.apply(&pool, id).await? {
        PatchOutcome::Updated(tag) => Ok(ApiJson(tag)),
        PatchOutcome::NotFound => Err(AppError::NotFound("tag")),
        PatchOutcome::NameTaken { existing_id } => Err(AppError::Duplicate {
            existing_id: existing_id.get(),
            msg: "a tag with that name already exists",
        }),
    }
}

// CHECKED:
// DELETE /api/tag/{id}   (page_tag / pending_tag rows cascade)
async fn delete_tag(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<TagId>,
) -> Result<StatusCode> {
    if !Tag::delete(&pool, id).await? {
        return Err(AppError::NotFound("tag"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeTagRequest {
    /// The surviving tag. Serialized as `"into"` to keep the existing wire format.
    #[serde(rename = "into")]
    pub target: TagId,
}

// CHECKED:
// POST /api/tag/{id}/merge   { "into": <tag_id> }
async fn merge_tag(
    State(pool): State<SqlitePool>,
    ApiPath(source): ApiPath<TagId>,
    ApiJson(req): ApiJson<MergeTagRequest>,
) -> Result<ApiJson<Tag>> {
    let merge =
        TagMerge::new(source, req.target).map_err(|e| AppError::BadRequest(e.to_string()))?;

    let target = Tag::merge(&pool, merge).await?.or_not_found("tag")?;
    Ok(ApiJson(target))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/tag", get(list_tags).post(create_tag))
        .route(
            "/tag/{id}",
            axum::routing::patch(patch_tag).delete(delete_tag),
        )
        .route("/tag/{id}/merge", post(merge_tag))
}
