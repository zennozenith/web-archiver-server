//! Pending-queue use-cases. Every queue is scoped to its owner.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::{
    db::{
        self, Database,
        create::CreateOutcome,
        cursor::{Cursor, CursorPage},
        pending::{PendingId, PendingResponse, PendingRow, PendingStatus, PendingUpdate},
    },
    domain::{TagName, parse_tag_names},
    error::{AppError, OptionNotFound as _, Resource, Result},
    identity::{AuthUser, Permission},
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
    Ok(if PendingRow::exists(pool, owner, id).await? {
        AppError::Conflict(conflict_message.into())
    } else {
        AppError::NotFound(Resource::PendingItem)
    })
}

pub(crate) async fn list(
    pool: &Database,
    actor: &AuthUser,
    status: Option<PendingStatus>,
    cursor: Option<Cursor>,
    limit: Option<Limit>,
) -> Result<CursorPage<PendingResponse>> {
    actor.require(Permission::ARCHIVE_READ)?;
    Ok(PendingResponse::list(
        pool.reader(),
        actor.id,
        status,
        cursor,
        limit.unwrap_or(DEFAULT_LIST_LIMIT),
    )
    .await?)
}

pub(crate) async fn get(
    pool: &Database,
    actor: &AuthUser,
    id: PendingId,
) -> Result<PendingResponse> {
    actor.require(Permission::ARCHIVE_READ)?;
    PendingResponse::get(pool.reader(), actor.id, id).await
}

pub(crate) async fn create(
    pool: &Database,
    actor: &AuthUser,
    url: &str,
    tags: &[String],
) -> Result<PendingResponse> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let (url, tags) = parse_new(url, tags)?;

    let mut tx = pool.begin_write().await?;
    let id = match PendingRow::enqueue(&mut tx, actor.id, &url, &tags).await? {
        CreateOutcome::Created(id) => id,
        CreateOutcome::Existing(existing_id) => {
            return Err(AppError::Duplicate {
                existing_id: existing_id.get(),
                msg: "url is already in the pending queue",
            });
        }
    };
    tx.commit().await?;

    PendingResponse::get(pool.reader(), actor.id, id).await
}

#[expect(clippy::too_many_arguments, reason = "mirrors the API patch fields")]
pub(crate) async fn patch(
    pool: &Database,
    actor: &AuthUser,
    id: PendingId,
    status: Option<PendingStatus>,
    url: Option<&str>,
    tags: Option<&[String]>,
) -> Result<PendingResponse> {
    actor.require(Permission::ARCHIVE_WRITE)?;

    if status == Some(PendingStatus::InProgress) {
        return Err(AppError::Invalid {
            code: "invalid_status",
            message: "use POST /api/pending/claim to start work on an item".into(),
        });
    }

    let tags = tags.map(parse_tag_names).transpose()?;
    let url = url.map(canonicalize_url).transpose()?.map(|url| url.url);

    let mut tx = pool.begin_write().await?;
    let current = PendingRow::status_of(&mut *tx, actor.id, id)
        .await?
        .or_not_found(Resource::PendingItem)?;

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

    PendingResponse::get(pool.reader(), actor.id, id).await
}

pub(crate) async fn retry(
    pool: &Database,
    actor: &AuthUser,
    id: PendingId,
) -> Result<PendingResponse> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if !PendingRow::retry(pool.writer(), actor.id, id).await? {
        return Err(conflict_or_not_found(
            pool.reader(),
            actor.id,
            id,
            "only failed items can be retried",
        )
        .await?);
    }
    PendingResponse::get(pool.reader(), actor.id, id).await
}

/// Atomically claims the actor's oldest queued item.
pub(crate) async fn claim(pool: &Database, actor: &AuthUser) -> Result<Option<PendingResponse>> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let Some(id) = PendingRow::claim_next(pool.writer(), Some(actor.id), now_ms()).await? else {
        return Ok(None);
    };
    Ok(PendingResponse::load(pool.reader(), actor.id, id).await?)
}

pub(crate) async fn fail(
    pool: &Database,
    actor: &AuthUser,
    id: PendingId,
    error: &str,
) -> Result<PendingResponse> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let error: String = error.chars().take(MAX_ERROR_CHARS).collect();

    if !PendingRow::mark_failed(pool.writer(), actor.id, id, &error).await? {
        return Err(
            conflict_or_not_found(pool.reader(), actor.id, id, "item is not in progress").await?,
        );
    }
    PendingResponse::get(pool.reader(), actor.id, id).await
}

pub(crate) async fn delete(pool: &Database, actor: &AuthUser, id: PendingId) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if !PendingRow::delete(pool.writer(), actor.id, id).await? {
        return Err(conflict_or_not_found(
            pool.reader(),
            actor.id,
            id,
            "item is currently being archived",
        )
        .await?);
    }
    Ok(())
}

pub(crate) async fn clear(pool: &Database, actor: &AuthUser, status: PendingStatus) -> Result<u64> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if status == PendingStatus::InProgress {
        return Err(AppError::BadRequest(
            "cannot clear in_progress items".into(),
        ));
    }
    Ok(PendingRow::delete_all_with_status(pool.writer(), actor.id, status).await?)
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
pub(crate) struct BulkItemResult {
    pub url: String,
    pub status: BulkItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<PendingId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct BulkCreateReport {
    pub created: usize,
    pub duplicates: usize,
    pub invalid: usize,
    pub results: Vec<BulkItemResult>,
}

/// One bad item does not fail the batch; DB errors roll back everything.
pub(crate) async fn bulk_create(
    pool: &Database,
    actor: &AuthUser,
    items: &[(String, Vec<String>)],
) -> Result<BulkCreateReport> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if items.is_empty() || items.len() > MAX_BULK_ITEMS {
        return Err(AppError::BadRequest(format!(
            "items must contain 1..={MAX_BULK_ITEMS} entries"
        )));
    }

    let mut tx = pool.begin_write().await?;
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
                let outcome = PendingRow::enqueue(&mut tx, actor.id, &url, &tags).await?;
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

pub(crate) async fn counts(pool: &Database, actor: &AuthUser) -> Result<PendingCounts> {
    actor.require(Permission::ARCHIVE_READ)?;
    let read = pool.reader();
    let (queued, in_progress, failed) = tokio::try_join!(
        db::pending::PendingRow::count_with_status(read, actor.id, PendingStatus::Queued),
        db::pending::PendingRow::count_with_status(read, actor.id, PendingStatus::InProgress),
        db::pending::PendingRow::count_with_status(read, actor.id, PendingStatus::Failed),
    )?;
    Ok(PendingCounts {
        queued,
        in_progress,
        failed,
        total: queued.saturating_add(in_progress).saturating_add(failed),
    })
}
