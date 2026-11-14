//! Pending queue pages and actions.

use askama::Template;
use axum::{
    Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    auth::{AuthUser, Permission},
    db::pending::{PendingId, PendingStatus},
    error::Result,
    http::{
        extract::body::{AppForm, AppPath, AppQuery},
        htmx::{Fragments, Render, ToastKind},
        web::{
            dashboard::LibraryDashboard,
            viewmodels::archive::PendingView,
            viewmodels::{Section, Shell},
        },
    },
    service,
    util::limit::Limit,
};

#[derive(Debug, Deserialize)]
pub(super) struct PendingQuery {
    pub status: Option<PendingStatus>,
}

#[derive(Debug, Template)]
#[template(path = "archive/pending.templ.html")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "view model flags drive template conditionals"
)]
pub(super) struct PendingTpl {
    shell: Shell,
    pending: Vec<PendingView>,
    counts: service::pending::PendingCounts,
    all_active: bool,
    queued_active: bool,
    in_progress_active: bool,
    failed_active: bool,
    can_write: bool,
}

pub(super) async fn index(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<PendingQuery>,
) -> Result<Render<PendingTpl>> {
    let items = service::pending::list(
        &ctx,
        &user,
        query.status,
        None,
        Some(Limit::from_const(100)),
    )
    .await?;
    let counts = service::pending::counts(&ctx, &user).await?;

    Ok(Render(PendingTpl {
        shell: Shell::new::<LibraryDashboard>(&user, Section::Pending),
        pending: items.items.into_iter().map(PendingView::from).collect(),
        all_active: query.status.is_none(),
        queued_active: query.status == Some(PendingStatus::Queued),
        in_progress_active: query.status == Some(PendingStatus::InProgress),
        failed_active: query.status == Some(PendingStatus::Failed),
        counts,
        can_write: user.can(Permission::ARCHIVE_WRITE),
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct CreatePendingForm {
    pub url: String,
    #[serde(default)]
    pub tags: String,
}

fn split_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) async fn create(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppForm(form): AppForm<CreatePendingForm>,
) -> Result<Fragments> {
    service::pending::create(&ctx, &user, &form.url, &split_tags(&form.tags)).await?;
    Fragments::new().toast(ToastKind::Success, "Added to the queue")
}

#[derive(Debug, Deserialize)]
pub(super) struct BulkForm {
    pub items: String,
}

/// One URL per line. Tags follow `|` (or the first space), separated by commas
/// or whitespace.
fn parse_bulk(raw: &str) -> Vec<(String, Vec<String>)> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            if let Some((url, tags)) = line.split_once('|') {
                return (url.trim().to_owned(), split_tags(tags));
            }
            let mut parts = line.split_whitespace();
            let url = parts.next().unwrap_or_default().to_owned();
            let tags = parts
                .flat_map(|part| part.split(','))
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect();
            (url, tags)
        })
        .collect()
}

pub(super) async fn bulk(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppForm(form): AppForm<BulkForm>,
) -> Result<Fragments> {
    let items = parse_bulk(&form.items);
    let report = service::pending::bulk_create(&ctx, &user, &items).await?;
    Fragments::new().toast(
        ToastKind::Success,
        &format!(
            "Added {} URLs ({} duplicates, {} invalid)",
            report.created, report.duplicates, report.invalid
        ),
    )
}

#[derive(Debug, Deserialize)]
pub(super) struct PatchPendingForm {
    pub url: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub status: Option<PendingStatus>,
}

pub(super) async fn update(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
    AppForm(form): AppForm<PatchPendingForm>,
) -> Result<Fragments> {
    service::pending::patch(
        &ctx,
        &user,
        id,
        form.status,
        Some(&form.url),
        Some(&split_tags(&form.tags)),
    )
    .await?;
    Fragments::new().toast(ToastKind::Success, "Queue item updated")
}

pub(super) async fn remove(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
) -> Result<Fragments> {
    service::pending::delete(&ctx, &user, id).await?;
    Fragments::new().toast(ToastKind::Success, "Removed from the queue")
}

pub(super) async fn retry(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PendingId>,
) -> Result<Fragments> {
    service::pending::retry(&ctx, &user, id).await?;
    Fragments::new().toast(ToastKind::Success, "Queued for retry")
}

#[derive(Debug, Deserialize)]
pub(super) struct ClearQuery {
    pub status: PendingStatus,
}

pub(super) async fn clear(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<ClearQuery>,
) -> Result<Fragments> {
    let deleted = service::pending::clear(&ctx, &user, query.status).await?;
    Fragments::new().toast(
        ToastKind::Success,
        &format!("Deleted {deleted} queued items"),
    )
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/pending", get(index).post(create).delete(clear))
        .route("/pending/bulk", post(bulk))
        .route("/pending/{id}", axum::routing::patch(update).delete(remove))
        .route("/pending/{id}/retry", post(retry))
}
