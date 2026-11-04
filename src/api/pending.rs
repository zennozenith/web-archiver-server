use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    db::{
        helper::{CreateOutcome, Cursor, CursorPage},
        pending::{PendingId, PendingItem, PendingRecord, PendingStatus, PendingUpdate},
    },
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath, ApiQuery, comma_separated_or_seq, opt_comma_separated_or_seq},
    limit::Limit,
    tag::{TagName, parse_names},
    url_canon::{CanonicalUrl, canonicalize_url},
};

const ITEM: &str = "pending item";
const DEFAULT_LIST_LIMIT: Limit = Limit::from_const(50);
const MAX_BULK_ITEMS: usize = 500;
const MAX_ERROR_CHARS: usize = 2000;

/// Body of `POST /api/pending` and each entry of `POST /api/pending/bulk`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatePendingRequest {
    pub url: String,
    /// Accepts either `["a", "b"]` or `"a,b"`.
    #[serde(default, deserialize_with = "comma_separated_or_seq")]
    pub tags: Vec<String>,
}

/// Body of `PATCH /api/pending/{id}`. Absent fields are left unchanged.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PatchPendingRequest {
    /// Replaces the whole tag set when present.
    #[serde(default, deserialize_with = "opt_comma_separated_or_seq")]
    pub tags: Option<Vec<String>>,
    pub status: Option<PendingStatus>,
}

/// Body of `POST /api/pending/{id}/fail`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FailPendingRequest {
    pub error: String,
}

// ── Shared helpers ──────────────────────────────────────────────────────────

// CHECKED:
async fn load_item(pool: &SqlitePool, id: PendingId) -> Result<ApiJson<PendingItem>> {
    let item = PendingItem::load(pool, id).await?.or_not_found(ITEM)?;
    Ok(ApiJson(item))
}

// CHECKED:
/// After a guarded update touched no rows: 409 if the item exists (wrong
/// state), 404 if it does not.
async fn conflict_or_not_found(
    pool: &SqlitePool,
    id: PendingId,
    conflict_message: &str,
) -> Result<AppError> {
    Ok(if PendingRecord::exists(pool, id).await? {
        AppError::Conflict(conflict_message.into())
    } else {
        AppError::NotFound(ITEM)
    })
}

fn parse_new_item(item: &CreatePendingRequest) -> Result<(CanonicalUrl, Vec<TagName>)> {
    Ok((canonicalize_url(&item.url)?, parse_names(&item.tags)?))
}

// ── GET /api/pending ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListPendingQuery {
    pub status: Option<PendingStatus>,
    pub limit: Option<Limit>,
    pub cursor: Option<Cursor>,
}

// CHECKED:
pub(super) async fn list_pending(
    State(pool): State<SqlitePool>,
    ApiQuery(query): ApiQuery<ListPendingQuery>,
) -> Result<ApiJson<CursorPage<PendingItem>>> {
    let page = PendingItem::list(
        &pool,
        query.status,
        query.cursor,
        query.limit.unwrap_or(DEFAULT_LIST_LIMIT),
    )
    .await?;
    Ok(ApiJson(page))
}

// ── POST /api/pending ───────────────────────────────────────────────────────
pub(super) async fn create_pending(
    State(pool): State<SqlitePool>,
    ApiJson(req): ApiJson<CreatePendingRequest>,
) -> Result<(StatusCode, ApiJson<PendingItem>)> {
    let (url, tags) = parse_new_item(&req)?;

    let mut tx = pool.begin().await?;
    let id = match PendingRecord::enqueue(&mut tx, &url, &tags).await? {
        CreateOutcome::Created(id) => id,
        CreateOutcome::Existing(existing_id) => {
            return Err(AppError::Duplicate {
                existing_id: existing_id.get(),
                msg: "url is already in the pending queue",
            });
        }
    };
    tx.commit().await?;

    Ok((StatusCode::CREATED, load_item(&pool, id).await?))
}

// ── POST /api/pending/bulk ──────────────────────────────────────────────────
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BulkCreateRequest {
    pub items: Vec<CreatePendingRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum BulkItemStatus {
    Created,
    Duplicate,
    Invalid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BulkItemResult {
    url: String,
    status: BulkItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<PendingId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl BulkItemResult {
    fn invalid(url: String, error: &AppError) -> Self {
        Self {
            url,
            status: BulkItemStatus::Invalid,
            id: None,
            error: Some(error.to_string()),
        }
    }

    fn from_outcome(url: String, outcome: CreateOutcome<PendingId>) -> Self {
        let status = if outcome.is_created() {
            BulkItemStatus::Created
        } else {
            BulkItemStatus::Duplicate
        };
        Self {
            url,
            status,
            id: Some(outcome.into_inner()),
            error: None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BulkCreateResponse {
    created: usize,
    duplicates: usize,
    invalid: usize,
    results: Vec<BulkItemResult>,
}

impl BulkCreateResponse {
    fn from_results(results: Vec<BulkItemResult>) -> Self {
        let count = |status| results.iter().filter(|r| r.status == status).count();
        Self {
            created: count(BulkItemStatus::Created),
            duplicates: count(BulkItemStatus::Duplicate),
            invalid: count(BulkItemStatus::Invalid),
            results,
        }
    }
}

/// One bad item doesn't fail the batch; DB errors roll back everything.
pub(super) async fn bulk_create_pending(
    State(pool): State<SqlitePool>,
    ApiJson(req): ApiJson<BulkCreateRequest>,
) -> Result<ApiJson<BulkCreateResponse>> {
    if req.items.is_empty() || req.items.len() > MAX_BULK_ITEMS {
        return Err(AppError::BadRequest(format!(
            "items must contain 1..={MAX_BULK_ITEMS} entries"
        )));
    }

    let mut tx = pool.begin().await?;
    let mut results = Vec::with_capacity(req.items.len());

    for item in req.items {
        let result = match parse_new_item(&item) {
            Err(error) => BulkItemResult::invalid(item.url, &error),
            Ok((url, tags)) => {
                let outcome = PendingRecord::enqueue(&mut tx, &url, &tags).await?;
                BulkItemResult::from_outcome(url.url, outcome)
            }
        };
        results.push(result);
    }
    tx.commit().await?;

    Ok(ApiJson(BulkCreateResponse::from_results(results)))
}

// ── GET /api/pending/{id} ───────────────────────────────────────────────────

pub(super) async fn get_pending(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PendingId>,
) -> Result<ApiJson<PendingItem>> {
    load_item(&pool, id).await
}

// ── PATCH /api/pending/{id} ─────────────────────────────────────────────────

pub(super) async fn patch_pending(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PendingId>,
    ApiJson(req): ApiJson<PatchPendingRequest>,
) -> Result<ApiJson<PendingItem>> {
    if req.status == Some(PendingStatus::InProgress) {
        return Err(AppError::Unprocessable {
            code: "invalid_status",
            message: "use POST /api/pending/claim to start work on an item".into(),
        });
    }
    let tags = req.tags.as_deref().map(parse_names).transpose()?;

    let mut tx = pool.begin().await?;

    let current = PendingRecord::status_of(&mut *tx, id)
        .await?
        .ok_or(AppError::NotFound(ITEM))?;

    if req.status.is_some() && current == PendingStatus::InProgress {
        return Err(AppError::Conflict(
            "item is currently being archived".into(),
        ));
    }

    PendingUpdate {
        status: req.status,
        tags: tags.as_deref(),
    }
    .apply(&mut tx, id)
    .await?;

    tx.commit().await?;

    load_item(&pool, id).await
}

// ── POST /api/pending/{id}/retry ────────────────────────────────────────────

pub(super) async fn retry_pending(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PendingId>,
) -> Result<ApiJson<PendingItem>> {
    if !PendingRecord::retry(&pool, id).await? {
        return Err(conflict_or_not_found(&pool, id, "only failed items can be retried").await?);
    }
    load_item(&pool, id).await
}

// ── POST /api/pending/claim ─────────────────────────────────────────────────

/// Atomically take the oldest queued item. 204 when the queue is empty.
pub(super) async fn claim_pending(State(pool): State<SqlitePool>) -> Result<Response> {
    let Some(id) = PendingRecord::claim_next(&pool).await? else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    Ok(load_item(&pool, id).await?.into_response())
}

// ── POST /api/pending/{id}/fail ─────────────────────────────────────────────

pub(super) async fn fail_pending(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PendingId>,
    ApiJson(req): ApiJson<FailPendingRequest>,
) -> Result<ApiJson<PendingItem>> {
    let error: String = req.error.chars().take(MAX_ERROR_CHARS).collect();

    if !PendingRecord::mark_failed(&pool, id, &error).await? {
        return Err(conflict_or_not_found(&pool, id, "item is not in progress").await?);
    }
    load_item(&pool, id).await
}

// ── DELETE /api/pending/{id} ────────────────────────────────────────────────
// Refuses to delete an item a worker is currently archiving (409).

pub(super) async fn delete_pending(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<PendingId>,
) -> Result<StatusCode> {
    if !PendingRecord::delete(&pool, id).await? {
        return Err(conflict_or_not_found(&pool, id, "item is currently being archived").await?);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── DELETE /api/pending?status=failed|queued ────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ClearPendingQuery {
    status: PendingStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ClearPendingResponse {
    deleted: u64,
}

pub(super) async fn clear_pending(
    State(pool): State<SqlitePool>,
    ApiQuery(query): ApiQuery<ClearPendingQuery>,
) -> Result<ApiJson<ClearPendingResponse>> {
    if query.status == PendingStatus::InProgress {
        return Err(AppError::BadRequest(
            "cannot clear in_progress items".into(),
        ));
    }
    let deleted = PendingRecord::delete_all_with_status(&pool, query.status).await?;
    Ok(ApiJson(ClearPendingResponse { deleted }))
}
