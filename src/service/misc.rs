//! Health, global archive statistics and maintenance.

use crate::{
    app::AppState,
    db::stats::{self, GCStat, Stats},
    error::Result,
    identity::{AuthUser, Permission},
};

pub(crate) async fn health(state: &AppState) -> Result<()> {
    Ok(stats::health(&state.db).await?)
}

/// Global installation statistics; only archive administrators may see them.
pub(crate) async fn stats(state: &AppState, actor: &AuthUser) -> Result<Stats> {
    actor.require(Permission::ARCHIVE_ADMIN)?;
    Ok(stats::stats(&state.db).await?)
}

/// Deletes unreferenced blobs and stale temporary files.
pub(crate) async fn gc(state: &AppState, actor: &AuthUser) -> Result<GCStat> {
    actor.require(Permission::ARCHIVE_ADMIN)?;
    stats::gc_content(&state.db, &state.blobs).await
}
