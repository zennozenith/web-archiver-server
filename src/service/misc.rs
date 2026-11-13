//! Health, global archive statistics and maintenance.

use crate::{
    app::AppState,
    auth::{AuthUser, Permission},
    db::stats::{self, GCStat, Stats},
    error::Result,
};

pub(crate) async fn health(ctx: &AppState) -> Result<()> {
    Ok(stats::health(&ctx.db).await?)
}

/// Global installation statistics; only archive administrators may see them.
pub(crate) async fn stats(ctx: &AppState, actor: &AuthUser) -> Result<Stats> {
    actor.require(Permission::ARCHIVE_ADMIN)?;
    Ok(stats::stats(&ctx.db).await?)
}

/// Deletes unreferenced blobs and stale temporary files.
pub(crate) async fn gc(ctx: &AppState, actor: &AuthUser) -> Result<GCStat> {
    actor.require(Permission::ARCHIVE_ADMIN)?;
    stats::gc_content(&ctx.db, &ctx.store).await
}
