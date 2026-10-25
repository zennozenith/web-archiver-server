use axum::{
    extract::State,
    response::{IntoResponse, Response},
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    db::{self, pending::Inserted},
    dto::{CreatePending, FailPending, Page, PatchPending, PendingDto},
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath, ApiQuery},
    models::PendingStatus,
    tag::clean_tags,
    url_canon::canonicalize_url,
};

// ── GET /api/pending ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListPendingQuery {
    pub status: Option<PendingStatus>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

pub async fn list_pending_handler(
    State(pool): State<SqlitePool>,
    ApiQuery(q): ApiQuery<ListPendingQuery>,
) -> Result<ApiJson<Page<PendingDto>>> {
    let after = q
        .cursor
        .as_deref()
        .map(|c| {
            c.parse::<i64>()
                .map_err(|_| AppError::BadRequest("invalid cursor".into()))
        })
        .transpose()?;
    Ok(ApiJson(
        db::pending::list_pending(&pool, q.status, after, q.limit.unwrap_or(50)).await?,
    ))
}

// ── POST /api/pending ───────────────────────────────────────────────────────

pub async fn create_pending(
    State(pool): State<SqlitePool>,
    ApiJson(req): ApiJson<CreatePending>,
) -> Result<(StatusCode, ApiJson<PendingDto>)> {
    let canon = canonicalize_url(&req.url)?;
    let tags = clean_tags(&req.tags)?;

    let mut tx = pool.begin().await?;
    let id = match db::pending::insert_pending(&mut tx, &canon, req.title.as_deref(), &tags).await?
    {
        db::pending::Inserted::New(id) => id,
        db::pending::Inserted::Existing(existing_id) => {
            return Err(AppError::DuplicatePending { existing_id });
        }
    };
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        ApiJson(
            db::pending::load_pending(&pool, id)
                .await?
                .or_not_found("pending item")?,
        ),
    ))
}

// ── POST /api/pending/bulk ──────────────────────────────────────────────────

const MAX_BULK: usize = 500;

#[derive(Debug, Deserialize)]
pub struct BulkCreate {
    pub items: Vec<CreatePending>,
}

#[derive(Serialize)]
pub struct BulkItemResult {
    url: String,
    status: &'static str, // created | duplicate | invalid
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
pub struct BulkResponse {
    created: usize,
    duplicates: usize,
    invalid: usize,
    results: Vec<BulkItemResult>,
}

/// One bad item doesn't fail the batch; DB errors roll back everything.
pub async fn bulk_create_pending(
    State(pool): State<SqlitePool>,
    ApiJson(req): ApiJson<BulkCreate>,
) -> Result<ApiJson<BulkResponse>> {
    if req.items.is_empty() || req.items.len() > MAX_BULK {
        return Err(AppError::BadRequest(format!(
            "items must contain 1..={MAX_BULK} entries"
        )));
    }

    let mut tx = pool.begin().await?;
    let mut results = Vec::with_capacity(req.items.len());
    let (mut created, mut duplicates, mut invalid) = (0, 0, 0);

    for item in req.items {
        let prepared = canonicalize_url(&item.url).and_then(|c| Ok((c, clean_tags(&item.tags)?)));
        let result = match prepared {
            Err(e) => {
                invalid += 1;
                BulkItemResult {
                    url: item.url,
                    status: "invalid",
                    id: None,
                    error: Some(e.to_string()),
                }
            }
            Ok((canon, tags)) => {
                match db::pending::insert_pending(&mut tx, &canon, item.title.as_deref(), &tags)
                    .await?
                {
                    Inserted::New(id) => {
                        created += 1;
                        BulkItemResult {
                            url: canon.url,
                            status: "created",
                            id: Some(id),
                            error: None,
                        }
                    }
                    Inserted::Existing(id) => {
                        duplicates += 1;
                        BulkItemResult {
                            url: canon.url,
                            status: "duplicate",
                            id: Some(id),
                            error: None,
                        }
                    }
                }
            }
        };
        results.push(result);
    }
    tx.commit().await?;

    Ok(ApiJson(BulkResponse {
        created,
        duplicates,
        invalid,
        results,
    }))
}

// ── GET /api/pending/{id} ───────────────────────────────────────────────────

pub async fn get_pending(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<PendingDto>> {
    Ok(ApiJson(
        db::pending::load_pending(&pool, id)
            .await?
            .or_not_found("pending item")?,
    ))
}

// ── PATCH /api/pending/{id} ─────────────────────────────────────────────────

pub async fn patch_pending(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(req): ApiJson<PatchPending>,
) -> Result<ApiJson<PendingDto>> {
    if req.status == Some(PendingStatus::InProgress) {
        return Err(AppError::Unprocessable {
            code: "invalid_status",
            message: "use POST /api/pending/claim to start work on an item".into(),
        });
    }
    let tags = req.tags.as_deref().map(clean_tags).transpose()?;

    let mut tx = pool.begin().await?;

    let current = db::pending::pending_status(&mut tx, id)
        .await?
        .ok_or(AppError::NotFound("pending item"))?;

    if req.status.is_some() && current == PendingStatus::InProgress {
        return Err(AppError::Conflict(
            "item is currently being archived".into(),
        ));
    }

    db::pending::update_pending(&mut tx, id, req.title.as_deref(), req.status, tags.as_ref())
        .await?;

    tx.commit().await?;

    Ok(ApiJson(
        db::pending::load_pending(&pool, id)
            .await?
            .or_not_found("pending item")?,
    ))
}

// ── POST /api/pending/{id}/retry ────────────────────────────────────────────

pub async fn retry_pending(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<ApiJson<PendingDto>> {
    let n = db::pending::retry_pending(&pool, id).await?;

    if n == 0 {
        return Err(
            db::helper::missing_pending_or(&pool, id, "only failed items can be retried").await?,
        );
    }
    Ok(ApiJson(
        db::pending::load_pending(&pool, id)
            .await?
            .or_not_found("pending item")?,
    ))
}

// ── POST /api/pending/claim ─────────────────────────────────────────────────

/// Atomically take the oldest queued item. 204 when the queue is empty.
/// A single UPDATE ... RETURNING means two workers can never claim the same row.
pub async fn claim_pending(State(pool): State<SqlitePool>) -> Result<Response> {
    let id: Option<i64> = db::pending::claim_pending(&pool).await?;

    Ok(match id {
        Some(id) => ApiJson(db::pending::load_pending(&pool, id).await?).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

// ── POST /api/pending/{id}/fail ─────────────────────────────────────────────

pub async fn fail_pending(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(req): ApiJson<FailPending>,
) -> Result<ApiJson<PendingDto>> {
    let error: String = req.error.chars().take(2000).collect();

    let n = db::pending::fail_pending(&pool, id, &error).await?;

    if n == 0 {
        return Err(db::helper::missing_pending_or(&pool, id, "item is not in progress").await?);
    }
    Ok(ApiJson(
        db::pending::load_pending(&pool, id)
            .await?
            .or_not_found("pending item")?,
    ))
}

// ── DELETE /api/pending/{id} ────────────────────────────────────────────────
// Refuses to delete an item a worker is currently archiving (409).

pub async fn delete_pending(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<StatusCode> {
    let n = db::pending::delete_pending(&pool, id).await?;
    if n == 0 {
        return Err(
            db::helper::missing_pending_or(&pool, id, "item is currently being archived").await?,
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── DELETE /api/pending?status=failed|queued ────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ClearQuery {
    status: PendingStatus,
}

pub async fn clear_pending(
    State(pool): State<SqlitePool>,
    ApiQuery(q): ApiQuery<ClearQuery>,
) -> Result<ApiJson<serde_json::Value>> {
    if q.status == PendingStatus::InProgress {
        return Err(AppError::BadRequest(
            "cannot clear in_progress items".into(),
        ));
    }
    let n = sqlx::query("DELETE FROM pending WHERE status = ?")
        .bind(q.status)
        .execute(&pool)
        .await?
        .rows_affected();
    Ok(ApiJson(serde_json::json!({ "deleted": n })))
}

// pub async fn delete_pending(
//     State(pool): State<SqlitePool>,
//     Path(id): Path<i64>,
// ) -> Result<StatusCode> {
//     let deleted = db::pending::delete_pending(&pool, id).await?;

//     if !deleted {
//         return Err(AppError::NotFound("pending item"));
//     }

//     Ok(StatusCode::NO_CONTENT)
// }
