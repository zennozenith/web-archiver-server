//! Collection use-cases. Collections are flat and belong to one user.

use crate::{
    app::AppState,
    auth::{AuthUser, Permission},
    db::collection::{CollectionId, CollectionInput, CollectionRecord},
    error::{AppError, Resource, Result},
};

/// Validated fields accepted when creating or editing a collection.
#[derive(Debug, Default)]
pub(crate) struct CollectionFields {
    pub name: String,
    pub emoji: String,
    pub description: String,
    pub color: String,
    pub public: bool,
}

impl CollectionFields {
    fn input(&self) -> Result<CollectionInput<'_>> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(AppError::invalid(
                "missing_name",
                "A collection needs a name.",
            ));
        }

        let emoji = self.emoji.trim();
        let color = self.color.trim();

        Ok(CollectionInput {
            name,
            emoji: if emoji.is_empty() { "📁" } else { emoji },
            description: self.description.trim(),
            color: if color.is_empty() { "#6d5dfc" } else { color },
            public: self.public,
        })
    }
}

pub(crate) async fn list(ctx: &AppState, actor: &AuthUser) -> Result<Vec<CollectionRecord>> {
    actor.require(Permission::ARCHIVE_READ)?;
    Ok(CollectionRecord::list(&ctx.db, actor.id).await?)
}

pub(crate) async fn get(
    ctx: &AppState,
    actor: &AuthUser,
    id: CollectionId,
) -> Result<CollectionRecord> {
    actor.require(Permission::ARCHIVE_READ)?;
    CollectionRecord::load(&ctx.db, actor.id, id)
        .await?
        .ok_or(AppError::NotFound(Resource::Collection))
}

pub(crate) async fn create(
    ctx: &AppState,
    actor: &AuthUser,
    fields: &CollectionFields,
) -> Result<CollectionRecord> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let input = fields.input()?;
    Ok(CollectionRecord::create(&ctx.db, actor.id, &input).await?)
}

pub(crate) async fn update(
    ctx: &AppState,
    actor: &AuthUser,
    id: CollectionId,
    fields: &CollectionFields,
) -> Result<CollectionRecord> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let input = fields.input()?;
    CollectionRecord::update(&ctx.db, actor.id, id, &input)
        .await?
        .ok_or(AppError::NotFound(Resource::Collection))
}

pub(crate) async fn delete(ctx: &AppState, actor: &AuthUser, id: CollectionId) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if CollectionRecord::delete(&ctx.db, actor.id, id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Collection))
    }
}

pub(crate) async fn set_public(
    ctx: &AppState,
    actor: &AuthUser,
    id: CollectionId,
    public: bool,
) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if CollectionRecord::set_public(&ctx.db, actor.id, id, public).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Collection))
    }
}

/// Public collection metadata. No sign-in required; only public rows are visible.
pub(crate) async fn public(ctx: &AppState, id: CollectionId) -> Result<CollectionRecord> {
    CollectionRecord::load_public(&ctx.db, id)
        .await?
        .ok_or(AppError::NotFound(Resource::PublicCollection))
}

/// A public collection together with its links. No sign-in required.
pub(crate) async fn public_links(
    ctx: &AppState,
    id: CollectionId,
    limit: crate::util::limit::Limit,
) -> Result<(CollectionRecord, Vec<crate::db::page::PageItem>)> {
    let record = public(ctx, id).await?;
    let filter = crate::db::page::PageFilter {
        owner_user_id: record.owner_user_id,
        collection_id: Some(id),
        limit,
        ..Default::default()
    };
    let items = crate::db::page::PageItem::list(&ctx.db, &filter).await?;
    Ok((record, items.items))
}
