//! Capture use-cases: metadata, blobs and sub-resources.

use sqlx::SqlitePool;

use crate::{
    db::{
        capture::{self, CaptureAsset, CaptureBlob, CaptureDetail, CaptureId},
        content::{Content, ContentHead},
        page::PageId,
    },
    error::{AppError, Resource, Result},
    identity::{AuthUser, Permission},
    infra::Blake3Hash,
};

pub(crate) async fn detail(
    pool: &SqlitePool,
    actor: &AuthUser,
    id: CaptureId,
) -> Result<CaptureDetail> {
    actor.require(Permission::ARCHIVE_READ)?;
    CaptureDetail::load(pool, actor.id, id)
        .await?
        .ok_or(AppError::NotFound(Resource::Capture))
}

pub(crate) async fn delete(pool: &SqlitePool, actor: &AuthUser, id: CaptureId) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if capture::delete(pool, actor.id, id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Capture))
    }
}

pub(crate) async fn blob(
    pool: &SqlitePool,
    actor: &AuthUser,
    id: CaptureId,
    blob: CaptureBlob,
) -> Result<Content> {
    actor.require(Permission::ARCHIVE_READ)?;
    Content::for_capture(pool, actor.id, id, blob)
        .await?
        .ok_or(AppError::NotFound(Resource::CaptureContent))
}

pub(crate) async fn page_thumbnail(
    pool: &SqlitePool,
    actor: &AuthUser,
    page_id: PageId,
) -> Result<Content> {
    actor.require(Permission::ARCHIVE_READ)?;
    Content::for_page(pool, actor.id, page_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Thumbnail))
}

/// Latest capture of the page that has `blob`, used for redirects.
pub(crate) async fn latest_with_blob(
    pool: &SqlitePool,
    actor: &AuthUser,
    page_id: PageId,
    blob: CaptureBlob,
) -> Result<CaptureId> {
    actor.require(Permission::ARCHIVE_READ)?;
    blob.latest_capture_id(pool, actor.id, page_id)
        .await?
        .ok_or(AppError::NotFound(Resource::CaptureContent))
}

pub(crate) async fn assets(
    pool: &SqlitePool,
    actor: &AuthUser,
    capture_id: CaptureId,
) -> Result<Vec<CaptureAsset>> {
    actor.require(Permission::ARCHIVE_READ)?;
    Ok(CaptureAsset::list(pool, actor.id, capture_id).await?)
}

pub(crate) async fn latest_assets(
    pool: &SqlitePool,
    actor: &AuthUser,
    page_id: PageId,
) -> Result<Vec<CaptureAsset>> {
    actor.require(Permission::ARCHIVE_READ)?;
    CaptureAsset::for_latest_capture(pool, actor.id, page_id)
        .await?
        .ok_or(AppError::NotFound(Resource::Page))
}

pub(crate) async fn asset_blob(
    pool: &SqlitePool,
    actor: &AuthUser,
    capture_id: CaptureId,
    content_id: i64,
) -> Result<Content> {
    actor.require(Permission::ARCHIVE_READ)?;
    Content::for_capture_asset(pool, actor.id, capture_id, content_id.into())
        .await?
        .ok_or(AppError::NotFound(Resource::Asset))
}

pub(crate) async fn content(
    pool: &SqlitePool,
    actor: &AuthUser,
    hash: &Blake3Hash,
) -> Result<Content> {
    actor.require(Permission::ARCHIVE_READ)?;
    let content = Content::by_hash(pool, hash)
        .await?
        .ok_or(AppError::NotFound(Resource::Content))?;
    if !Content::visible_to(pool, actor.id, content.id).await? {
        return Err(AppError::NotFound(Resource::Content));
    }
    Ok(content)
}

pub(crate) async fn content_head(
    pool: &SqlitePool,
    actor: &AuthUser,
    hash: &Blake3Hash,
) -> Result<ContentHead> {
    actor.require(Permission::ARCHIVE_READ)?;
    ContentHead::by_hash_visible(pool, actor.id, hash)
        .await?
        .ok_or(AppError::NotFound(Resource::Content))
}
