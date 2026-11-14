//! Pending-queue use-cases. Every queue is scoped to its owner.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::{
    app::AppState,
    auth::{AuthUser, Permission},
    db::{
        self,
        helper::{CreateOutcome, Cursor, CursorPage},
        pending::{PendingId, PendingItem, PendingRecord, PendingStatus, PendingUpdate},
    },
    error::{AppError, Resource, Result},
    infra::{TagName, parse_tag_names},
    util::{
        limit::Limit,
        time::now_ms,
        url_canon::{CanonicalUrl, canonicalize_url},
    },
};

const DEFAULT_LIST_LIMIT: Limit = Limit::from_const(50);
pub(crate) const MAX_BULK_ITEMS: usize = 500;
const MAX_ERROR_CHARS: usize = 2000;

fn parse_new(url: &str, tags: &[String]) -> Result<(CanonicalUrl, Vec<TagName>)> {
    Ok((canonicalize_url(url)?, parse_tag_names(tags)?))
}

async fn conflict_or_not_found(
    pool: &SqlitePool,
    owner: i64,
    id: PendingId,
    conflict_message: &str,
) -> Result<AppError> {
    Ok(if PendingRecord::exists(pool, owner, id).await? {
        AppError::Conflict(conflict_message.into())
    } else {
        AppError::NotFound(Resource::PendingItem)
    })
}

pub(crate) async fn list(
    ctx: &AppState,
    actor: &AuthUser,
    status: Option<PendingStatus>,
    cursor: Option<Cursor>,
    limit: Option<Limit>,
) -> Result<CursorPage<PendingItem>> {
    actor.require(Permission::ARCHIVE_READ)?;
    Ok(PendingItem::list(
        &ctx.db,
        actor.id,
        status,
        cursor,
        limit.unwrap_or(DEFAULT_LIST_LIMIT),
    )
    .await?)
}

pub(crate) async fn get(ctx: &AppState, actor: &AuthUser, id: PendingId) -> Result<PendingItem> {
    actor.require(Permission::ARCHIVE_READ)?;
    PendingItem::get(&ctx.db, actor.id, id).await
}

pub(crate) async fn create(
    ctx: &AppState,
    actor: &AuthUser,
    url: &str,
    tags: &[String],
) -> Result<PendingItem> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let (url, tags) = parse_new(url, tags)?;

    let mut tx = ctx.db.begin().await?;
    let id = match PendingRecord::enqueue(&mut tx, actor.id, &url, &tags).await? {
        CreateOutcome::Created(id) => id,
        CreateOutcome::Existing(existing_id) => {
            return Err(AppError::Duplicate {
                existing_id: existing_id.get(),
                msg: "url is already in the pending queue",
            });
        }
    };
    tx.commit().await?;

    PendingItem::get(&ctx.db, actor.id, id).await
}

#[expect(clippy::too_many_arguments, reason = "mirrors the API patch fields")]
pub(crate) async fn patch(
    ctx: &AppState,
    actor: &AuthUser,
    id: PendingId,
    status: Option<PendingStatus>,
    url: Option<&str>,
    tags: Option<&[String]>,
) -> Result<PendingItem> {
    actor.require(Permission::ARCHIVE_WRITE)?;

    if status == Some(PendingStatus::InProgress) {
        return Err(AppError::Invalid {
            code: "invalid_status",
            message: "use POST /api/pending/claim to start work on an item".into(),
        });
    }

    let tags = tags.map(parse_tag_names).transpose()?;
    let url = url.map(canonicalize_url).transpose()?.map(|url| url.url);

    let mut tx = ctx.db.begin().await?;
    let current = PendingRecord::status_of(&mut *tx, actor.id, id)
        .await?
        .ok_or(AppError::NotFound(Resource::PendingItem))?;

    if status.is_some() && current == PendingStatus::InProgress {
        return Err(AppError::Conflict(
            "item is currently being archived".into(),
        ));
    }

    PendingUpdate {
        status,
        url,
        tags: tags.as_deref(),
    }
    .apply(&mut tx, actor.id, id)
    .await?;
    tx.commit().await?;

    PendingItem::get(&ctx.db, actor.id, id).await
}

pub(crate) async fn retry(ctx: &AppState, actor: &AuthUser, id: PendingId) -> Result<PendingItem> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if !PendingRecord::retry(&ctx.db, actor.id, id).await? {
        return Err(conflict_or_not_found(
            &ctx.db,
            actor.id,
            id,
            "only failed items can be retried",
        )
        .await?);
    }
    PendingItem::get(&ctx.db, actor.id, id).await
}

/// Atomically claims the actor's oldest queued item.
pub(crate) async fn claim(ctx: &AppState, actor: &AuthUser) -> Result<Option<PendingItem>> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let Some(id) = PendingRecord::claim_next(&ctx.db, Some(actor.id), now_ms()).await? else {
        return Ok(None);
    };
    Ok(PendingItem::load(&ctx.db, actor.id, id).await?)
}

pub(crate) async fn fail(
    ctx: &AppState,
    actor: &AuthUser,
    id: PendingId,
    error: &str,
) -> Result<PendingItem> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let error: String = error.chars().take(MAX_ERROR_CHARS).collect();

    if !PendingRecord::mark_failed(&ctx.db, actor.id, id, &error).await? {
        return Err(conflict_or_not_found(&ctx.db, actor.id, id, "item is not in progress").await?);
    }
    PendingItem::get(&ctx.db, actor.id, id).await
}

pub(crate) async fn delete(ctx: &AppState, actor: &AuthUser, id: PendingId) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if !PendingRecord::delete(&ctx.db, actor.id, id).await? {
        return Err(conflict_or_not_found(
            &ctx.db,
            actor.id,
            id,
            "item is currently being archived",
        )
        .await?);
    }
    Ok(())
}

pub(crate) async fn clear(ctx: &AppState, actor: &AuthUser, status: PendingStatus) -> Result<u64> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if status == PendingStatus::InProgress {
        return Err(AppError::BadRequest(
            "cannot clear in_progress items".into(),
        ));
    }
    Ok(PendingRecord::delete_all_with_status(&ctx.db, actor.id, status).await?)
}

// ------------------------------------------------------------------ bulk add

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BulkItemStatus {
    Created,
    Duplicate,
    Invalid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkItemResult {
    pub url: String,
    pub status: BulkItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<PendingId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkCreateReport {
    pub created: usize,
    pub duplicates: usize,
    pub invalid: usize,
    pub results: Vec<BulkItemResult>,
}

/// One bad item does not fail the batch; DB errors roll back everything.
pub(crate) async fn bulk_create(
    ctx: &AppState,
    actor: &AuthUser,
    items: &[(String, Vec<String>)],
) -> Result<BulkCreateReport> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if items.is_empty() || items.len() > MAX_BULK_ITEMS {
        return Err(AppError::BadRequest(format!(
            "items must contain 1..={MAX_BULK_ITEMS} entries"
        )));
    }

    let mut tx = ctx.db.begin().await?;
    let mut results = Vec::with_capacity(items.len());

    for (url, tags) in items {
        let result = match parse_new(url, tags) {
            Err(error) => BulkItemResult {
                url: url.clone(),
                status: BulkItemStatus::Invalid,
                id: None,
                error: Some(error.to_string()),
            },
            Ok((url, tags)) => {
                let outcome = PendingRecord::enqueue(&mut tx, actor.id, &url, &tags).await?;
                BulkItemResult {
                    url: url.url,
                    status: if outcome.is_created() {
                        BulkItemStatus::Created
                    } else {
                        BulkItemStatus::Duplicate
                    },
                    id: Some(outcome.into_inner()),
                    error: None,
                }
            }
        };
        results.push(result);
    }
    tx.commit().await?;

    let count = |status| results.iter().filter(|r| r.status == status).count();
    Ok(BulkCreateReport {
        created: count(BulkItemStatus::Created),
        duplicates: count(BulkItemStatus::Duplicate),
        invalid: count(BulkItemStatus::Invalid),
        results,
    })
}

/// Counts per queue status for the queue page.
#[derive(Debug)]
pub(crate) struct PendingCounts {
    pub queued: i64,
    pub in_progress: i64,
    pub failed: i64,
    pub total: i64,
}

pub(crate) async fn counts(ctx: &AppState, actor: &AuthUser) -> Result<PendingCounts> {
    actor.require(Permission::ARCHIVE_READ)?;
    let (queued, in_progress, failed) = tokio::try_join!(
        db::pending::PendingRecord::count_with_status(&ctx.db, actor.id, PendingStatus::Queued),
        db::pending::PendingRecord::count_with_status(&ctx.db, actor.id, PendingStatus::InProgress),
        db::pending::PendingRecord::count_with_status(&ctx.db, actor.id, PendingStatus::Failed),
    )?;
    Ok(PendingCounts {
        queued,
        in_progress,
        failed,
        total: queued.saturating_add(in_progress).saturating_add(failed),
    })
}
