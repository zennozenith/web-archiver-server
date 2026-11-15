//! Capture use-cases: metadata, blobs and sub-resources.

use crate::{
    db::{
        Database,
        capture::{self, CaptureAsset, CaptureBlob, CaptureDetail, CaptureId},
        content::{Content, ContentHead},
        page::PageId,
    },
    error::{AppError, OptionNotFound as _, Resource, Result},
    identity::{AuthUser, Permission},
    infra::Blake3Hash,
};

pub(crate) async fn detail(
    pool: &Database,
    actor: &AuthUser,
    id: CaptureId,
) -> Result<CaptureDetail> {
    actor.require(Permission::ARCHIVE_READ)?;
    CaptureDetail::load(pool.reader(), actor.id, id)
        .await?
        .or_not_found(Resource::Capture)
}

pub(crate) async fn delete(pool: &Database, actor: &AuthUser, id: CaptureId) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if capture::delete(pool.writer(), actor.id, id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Capture))
    }
}

pub(crate) async fn blob(
    pool: &Database,
    actor: &AuthUser,
    id: CaptureId,
    blob: CaptureBlob,
) -> Result<Content> {
    actor.require(Permission::ARCHIVE_READ)?;
    Content::for_capture(pool.reader(), actor.id, id, blob)
        .await?
        .or_not_found(Resource::CaptureContent)
}

pub(crate) async fn page_thumbnail(
    pool: &Database,
    actor: &AuthUser,
    page_id: PageId,
) -> Result<Content> {
    actor.require(Permission::ARCHIVE_READ)?;
    Content::for_page(pool.reader(), actor.id, page_id)
        .await?
        .or_not_found(Resource::Thumbnail)
}

/// Latest capture of the page that has `blob`, used for redirects.
pub(crate) async fn latest_with_blob(
    pool: &Database,
    actor: &AuthUser,
    page_id: PageId,
    blob: CaptureBlob,
) -> Result<CaptureId> {
    actor.require(Permission::ARCHIVE_READ)?;
    blob.latest_capture_id(pool.reader(), actor.id, page_id)
        .await?
        .or_not_found(Resource::CaptureContent)
}

pub(crate) async fn assets(
    pool: &Database,
    actor: &AuthUser,
    capture_id: CaptureId,
) -> Result<Vec<CaptureAsset>> {
    actor.require(Permission::ARCHIVE_READ)?;
    Ok(CaptureAsset::list(pool.reader(), actor.id, capture_id).await?)
}

pub(crate) async fn latest_assets(
    pool: &Database,
    actor: &AuthUser,
    page_id: PageId,
) -> Result<Vec<CaptureAsset>> {
    actor.require(Permission::ARCHIVE_READ)?;
    CaptureAsset::for_latest_capture(pool.reader(), actor.id, page_id)
        .await?
        .or_not_found(Resource::Page)
}

pub(crate) async fn asset_blob(
    pool: &Database,
    actor: &AuthUser,
    capture_id: CaptureId,
    content_id: i64,
) -> Result<Content> {
    actor.require(Permission::ARCHIVE_READ)?;
    Content::for_capture_asset(pool.reader(), actor.id, capture_id, content_id.into())
        .await?
        .or_not_found(Resource::Asset)
}

pub(crate) async fn content(
    pool: &Database,
    actor: &AuthUser,
    hash: &Blake3Hash,
) -> Result<Content> {
    actor.require(Permission::ARCHIVE_READ)?;
    let content = Content::by_hash(pool.reader(), hash)
        .await?
        .or_not_found(Resource::Content)?;
    if !Content::visible_to(pool.reader(), actor.id, content.id).await? {
        return Err(AppError::NotFound(Resource::Content));
    }
    Ok(content)
}

pub(crate) async fn content_head(
    pool: &Database,
    actor: &AuthUser,
    hash: &Blake3Hash,
) -> Result<ContentHead> {
    actor.require(Permission::ARCHIVE_READ)?;
    ContentHead::by_hash_visible(pool.reader(), actor.id, hash)
        .await?
        .or_not_found(Resource::Content)
}
