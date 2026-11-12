use askama::Template;
use axum::{
    Form,
    extract::{Path, State},
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    db::{
        helper::CreateOutcome,
        pending::{PendingId, PendingItem, PendingRecord, PendingStatus},
    },
    error::{AppError, OptionExt, Result},
    extract::comma_separated_or_seq,
    htmx::{HtmxResponse, PartialSwap},
    tag,
    template::FeedbackTemplate,
    time::ago_timestamp,
    url_canon::canonicalize_url,
};

#[derive(Template)]
#[template(path = "archive/_pending_card.html")]
pub(super) struct PendingFragmentTemplate<'a> {
    pub id: i64,
    pub url: &'a str,
    pub status: PendingStatus,
    pub created_at: String,
    pub tags: String,
}

impl<'a> From<&'a PendingItem> for PendingFragmentTemplate<'a> {
    fn from(value: &'a PendingItem) -> Self {
        PendingFragmentTemplate {
            id: value.record.id.get(),
            url: &value.record.url,
            status: value.record.status,
            created_at: ago_timestamp(value.record.created_at),
            tags: Vec::from_iter(value.tags.iter().map(|v| v.name.clone())).join(", "),
        }
    }
}

#[derive(Template)]
#[template(path = "archive/pending.html")]
pub(super) struct PendingTemplate<'a> {
    pub id: i64,
    pub url: &'a str,
    pub status: PendingStatus,
    pub created_at: String,
    pub tags: String,
}

impl<'a> From<&'a PendingItem> for PendingTemplate<'a> {
    fn from(value: &'a PendingItem) -> Self {
        PendingTemplate {
            id: value.record.id.get(),
            url: &value.record.url,
            status: value.record.status,
            created_at: ago_timestamp(value.record.created_at),
            tags: Vec::from_iter(value.tags.iter().map(|v| v.name.clone())).join(", "),
        }
    }
}

/// Body of `POST /archive/pending`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreatePendingRequest {
    pub url: String,
    /// Accepts either `["a", "b"]` or `"a,b"`.
    #[serde(default, deserialize_with = "comma_separated_or_seq")]
    pub tags: Vec<String>,
}

pub(super) async fn create_pending(
    State(pool): State<SqlitePool>,
    Form(req): Form<CreatePendingRequest>,
) -> Result<HtmxResponse> {
    let canon = canonicalize_url(&req.url)?;
    let tags = tag::parse_names(&req.tags)?;

    let mut tx = pool.begin().await?;
    let id = match PendingRecord::enqueue(&mut tx, &canon, &tags).await? {
        CreateOutcome::Created(id) => id,
        CreateOutcome::Existing(existing_id) => {
            return Err(AppError::Duplicate {
                existing_id: existing_id.get(),
                msg: "url is already in the pending queue",
            });
        }
    };
    tx.commit().await?;

    let pending_dto = PendingItem::load(&pool, id)
        .await?
        .or_not_found("pending item")?;

    Ok(HtmxResponse::partials([
        // New card goes to the top of the list
        PartialSwap::tpl(
            "#pending-list",
            &PendingFragmentTemplate::from(&pending_dto),
        )?
        .swap("afterbegin"),
        // Clear any previous error message
        PartialSwap::new("#pending-feedback", "").swap("innerHTML"),
    ]))
}

pub(super) async fn delete_pending(
    State(pool): State<SqlitePool>,
    Path(id): Path<PendingId>,
) -> Result<HtmxResponse> {
    // Idempotent: if the row is already gone, the card should disappear anyway.
    PendingRecord::delete(&pool, id).await?;

    // 200 with an empty body, not 204. htmx 4 doesn't swap 204 responses,
    // so the card would never be removed.
    Ok(HtmxResponse::partials([
        PartialSwap::new(format!("#pending-{id}"), "").swap("outerHTML"),
        PartialSwap::tpl("#pending-feedback", &FeedbackTemplate::success("Removed"))?
            .swap("innerHTML"),
    ]))
}
