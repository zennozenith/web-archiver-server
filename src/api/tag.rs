use axum::{extract::State, http::StatusCode};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    db::{
        helpers::Inserted,
        tags::{TagRow, TagWithCount},
    },
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath},
    tag::{Color, Merge, Name},
};

#[derive(Debug, Deserialize)]
pub(super) struct CreateTag {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReplaceTags {
    pub tags: Vec<CreateTag>,
}

/// PATCH semantics: `color: null` clears, absent leaves unchanged.
#[derive(Debug, Deserialize)]
#[expect(
    clippy::option_option,
    reason = "distinguishes an absent field from an explicit null in PATCH payloads"
)]
pub(super) struct PatchTag {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub color: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MergeTag {
    pub into: i64,
}

#[expect(
    clippy::option_option,
    reason = "distinguishes an absent field from an explicit null in PATCH payloads"
)]
fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(d).map(Some)
}

/// Colors are palette keys like "red" or "deep-blue", never hex values.
pub(super) fn validate_color(color: Option<&str>) -> Result<Option<Color>> {
    color.map(str::trim).filter(|c| !c.is_empty()).map_or_else(
        || Ok(None),
        |c| {
            Color::new(c)
                .map(Some)
                .map_err(|e| AppError::invalid("invalid_color", e.to_string()))
        },
    )
}

pub(crate) fn clean_tag_name(raw: &str) -> Result<Name> {
    Name::new(raw).map_err(|e| AppError::invalid("invalid_tag", e.to_string()))
}

// GET /api/tags
pub(super) async fn list_tags(
    State(pool): State<SqlitePool>,
) -> Result<ApiJson<Vec<TagWithCount>>> {
    let list = TagWithCount::list(&pool).await?;
    Ok(ApiJson(list))
}

// POST /api/tags
pub(super) async fn create_tag(
    State(pool): State<SqlitePool>,
    ApiJson(req): ApiJson<CreateTag>,
) -> Result<(StatusCode, ApiJson<TagRow>)> {
    let name = clean_tag_name(&req.name)?;

    let color = validate_color(req.color.as_deref())?;

    let created = TagRow::create(&pool, name, color).await?;

    match created {
        Inserted::New(tag) => Ok((StatusCode::CREATED, ApiJson(tag))),
        Inserted::Existing(existing_id) => Err(AppError::DuplicateTag { existing_id }),
    }
}

// PATCH /api/tags/{id}   (color: null clears, absent keeps)
pub(super) async fn patch_tag(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(req): ApiJson<PatchTag>,
) -> Result<ApiJson<TagRow>> {
    let name = req.name.as_deref().map(clean_tag_name).transpose()?;
    let color = req
        .color
        .map(|v| validate_color(v.as_deref()))
        .transpose()?;

    let updated = crate::db::tags::PatchTag { name, color }
        .patch(&pool, id)
        .await?
        .or_not_found("tag")?;

    match updated {
        Inserted::New(tag) => Ok(ApiJson(tag)),
        Inserted::Existing(existing_id) => Err(AppError::DuplicateTag { existing_id }),
    }
}

// DELETE /api/tags/{id}   (url_tag / pending_tag rows cascade)
pub(super) async fn delete_tag(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<StatusCode> {
    let n = TagRow::delete(&pool, id).await?;
    if n == 0 {
        return Err(AppError::NotFound("tag"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// POST /api/tags/{id}/merge   { "into": <tag_id> }
pub(super) async fn merge_tag(
    State(pool): State<SqlitePool>,
    ApiPath(from): ApiPath<i64>,
    ApiJson(MergeTag { into }): ApiJson<MergeTag>,
) -> Result<ApiJson<TagRow>> {
    let merge = Merge::new(from, into).map_err(|v| AppError::BadRequest(v.to_string()))?;

    let Some(target) = TagRow::merge(&pool, merge).await? else {
        return Err(AppError::NotFound("tag"));
    };

    Ok(ApiJson(target))
}
