//! Tag use-cases. Tags belong to one user; every operation is owner-scoped.

use crate::{
    app::AppState,
    auth::{AuthUser, Permission},
    db::{
        helper::{CreateOutcome, FieldPatch},
        tag::{PatchOutcome, Tag, TagPatch, TagSummary},
    },
    error::{AppError, Resource, Result},
    infra::{TagColor, TagId, TagMerge, TagName, parse_optional_color},
};

fn color_patch(raw: FieldPatch<String>) -> Result<FieldPatch<TagColor>> {
    Ok(match raw {
        FieldPatch::Unchanged => FieldPatch::Unchanged,
        FieldPatch::Clear => FieldPatch::Clear,
        FieldPatch::Set(value) => {
            parse_optional_color(Some(&value))?.map_or(FieldPatch::Clear, FieldPatch::Set)
        }
    })
}

pub(crate) async fn list(ctx: &AppState, actor: &AuthUser) -> Result<Vec<TagSummary>> {
    actor.require(Permission::ARCHIVE_READ)?;
    Ok(TagSummary::list(&ctx.db, actor.id).await?)
}

pub(crate) async fn find(
    ctx: &AppState,
    actor: &AuthUser,
    name: &str,
) -> Result<Option<TagSummary>> {
    actor.require(Permission::ARCHIVE_READ)?;
    let name: TagName = name.parse()?;
    Ok(TagSummary::find_by_name(&ctx.db, actor.id, &name).await?)
}

pub(crate) async fn create(
    ctx: &AppState,
    actor: &AuthUser,
    name: &str,
    color: Option<&str>,
) -> Result<Tag> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let name: TagName = name.parse()?;
    let color = parse_optional_color(color)?;

    match Tag::create(&ctx.db, actor.id, &name, color.as_ref()).await? {
        CreateOutcome::Created(tag) => Ok(tag),
        CreateOutcome::Existing(tag) => Err(AppError::Duplicate {
            existing_id: tag.id.get(),
            msg: "a tag with that name already exists",
        }),
    }
}

pub(crate) async fn patch(
    ctx: &AppState,
    actor: &AuthUser,
    id: TagId,
    name: Option<&str>,
    color: FieldPatch<String>,
) -> Result<Tag> {
    actor.require(Permission::ARCHIVE_WRITE)?;

    let patch = TagPatch {
        name: name.map(str::parse).transpose()?,
        color: color_patch(color)?,
    };

    match patch.apply(&ctx.db, actor.id, id).await? {
        PatchOutcome::Updated(tag) => Ok(tag),
        PatchOutcome::NotFound => Err(AppError::NotFound(Resource::Tag)),
        PatchOutcome::NameTaken { existing_id } => Err(AppError::Duplicate {
            existing_id: existing_id.get(),
            msg: "a tag with that name already exists",
        }),
    }
}

pub(crate) async fn delete(ctx: &AppState, actor: &AuthUser, id: TagId) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if Tag::delete(&ctx.db, actor.id, id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Tag))
    }
}

pub(crate) async fn merge(
    ctx: &AppState,
    actor: &AuthUser,
    source: TagId,
    target: TagId,
) -> Result<Tag> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let merge = TagMerge::new(source, target).map_err(|e| AppError::BadRequest(e.to_string()))?;
    Tag::merge(&ctx.db, actor.id, merge)
        .await?
        .ok_or(AppError::NotFound(Resource::Tag))
}
