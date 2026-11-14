//! Collection use-cases. Collections are flat and belong to one user.

use sqlx::SqlitePool;

use crate::{
    db::collection::{CollectionId, CollectionInput, CollectionRecord},
    error::{AppError, OptionNotFound as _, Resource, Result},
    identity::{AuthUser, Permission},
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

pub(crate) async fn list(pool: &SqlitePool, actor: &AuthUser) -> Result<Vec<CollectionRecord>> {
    actor.require(Permission::ARCHIVE_READ)?;
    Ok(CollectionRecord::list(pool, actor.id).await?)
}

pub(crate) async fn get(
    pool: &SqlitePool,
    actor: &AuthUser,
    id: CollectionId,
) -> Result<CollectionRecord> {
    actor.require(Permission::ARCHIVE_READ)?;
    CollectionRecord::load(pool, actor.id, id)
        .await?
        .or_not_found(Resource::Collection)
}

pub(crate) async fn create(
    pool: &SqlitePool,
    actor: &AuthUser,
    fields: &CollectionFields,
) -> Result<CollectionRecord> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let input = fields.input()?;
    CollectionRecord::create(pool, actor.id, &input)
        .await
        .map_err(|e| AppError::unique(e, "a collection with that name already exists"))
}

pub(crate) async fn update(
    pool: &SqlitePool,
    actor: &AuthUser,
    id: CollectionId,
    fields: &CollectionFields,
) -> Result<CollectionRecord> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let input = fields.input()?;
    CollectionRecord::update(pool, actor.id, id, &input)
        .await
        .map_err(|e| AppError::unique(e, "a collection with that name already exists"))?
        .or_not_found(Resource::Collection)
}

pub(crate) async fn delete(pool: &SqlitePool, actor: &AuthUser, id: CollectionId) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if CollectionRecord::delete(pool, actor.id, id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Collection))
    }
}

pub(crate) async fn set_public(
    pool: &SqlitePool,
    actor: &AuthUser,
    id: CollectionId,
    public: bool,
) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if CollectionRecord::set_public(pool, actor.id, id, public).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Collection))
    }
}

/// Public collection metadata. No sign-in required; only public rows are visible.
pub(crate) async fn public(pool: &SqlitePool, id: CollectionId) -> Result<CollectionRecord> {
    CollectionRecord::load_public(pool, id)
        .await?
        .or_not_found(Resource::PublicCollection)
}

/// A public collection together with its links. No sign-in required.
pub(crate) async fn public_links(
    pool: &SqlitePool,
    id: CollectionId,
    limit: crate::util::limit::Limit,
) -> Result<(CollectionRecord, Vec<crate::db::page::PageItem>)> {
    let record = public(pool, id).await?;
    let filter = crate::db::page::PageFilter {
        owner_user_id: record.owner_user_id,
        collection_id: Some(id),
        limit,
        ..Default::default()
    };
    let items = crate::db::page::PageItem::list(pool, &filter).await?;
    Ok((record, items.items))
}
