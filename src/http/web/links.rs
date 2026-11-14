//! Link pages: listing, search fragment, detail and actions.

use askama::Template;
use axum::{
    Router,
    extract::{Multipart, State, multipart::MultipartRejection},
    http::HeaderMap,
    routing::{get, post, put},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    auth::{AuthUser, Permission},
    db::{
        collection::CollectionId,
        helper::FieldPatch,
        page::{PageId, UpdatePage},
    },
    error::{AppError, Result},
    http::{
        extract::body::{AppForm, AppPath, AppQuery},
        htmx::{Fragments, Render, ToastKind},
        multipart::parse_page_upload,
        web::{
            dashboard::LibraryDashboard,
            library::{self, LinkQuery},
            viewmodels::archive::{CaptureView, CollectionChoice, HighlightView, LinkCard},
            viewmodels::{Section, Shell},
        },
    },
    infra::{Image, ImageFormat},
    service,
    util::{limit::Limit, parse_bool},
};

#[derive(Debug, Template)]
#[template(path = "archive/links.templ.html")]
pub(super) struct LinksTpl {
    shell: Shell,
    links: Vec<LinkCard>,
    next_cursor: Option<String>,
    filters: library::LinkFilters,
    collections: Vec<CollectionChoice>,
    can_write: bool,
}

async fn page(
    ctx: &AppState,
    user: &crate::auth::AuthUser,
    query: &LinkQuery,
    section: Section,
) -> Result<LinksTpl, AppError> {
    let (links, next_cursor) = library::load(ctx, user, query).await?;
    Ok(LinksTpl {
        shell: Shell::new::<LibraryDashboard>(user, section),
        links,
        next_cursor,
        filters: query.filters(),
        collections: library::collection_choices(ctx, user, None).await?,
        can_write: user.can(Permission::ARCHIVE_WRITE),
    })
}

pub(super) async fn index(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<LinkQuery>,
) -> Result<Render<LinksTpl>> {
    Ok(Render(page(&ctx, &user, &query, Section::Links).await?))
}

#[derive(Debug, Template)]
#[template(path = "archive/link_grid.templ.html")]
pub(super) struct LinkGridTpl {
    links: Vec<LinkCard>,
    next_cursor: Option<String>,
    filters: library::LinkFilters,
}

pub(super) async fn search(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<LinkQuery>,
) -> Result<Render<LinkGridTpl>> {
    let filters = query.filters();
    let (links, next_cursor) = library::load(&ctx, &user, &query).await?;
    Ok(Render(LinkGridTpl {
        links,
        next_cursor,
        filters,
    }))
}

#[derive(Debug, Template)]
#[template(path = "archive/pinned.templ.html")]
pub(super) struct PinnedTpl {
    shell: Shell,
    links: Vec<LinkCard>,
}

pub(super) async fn pinned(
    State(ctx): State<AppState>,
    user: AuthUser,
) -> Result<Render<PinnedTpl>> {
    let query = LinkQuery {
        pinned_only: true,
        ..Default::default()
    };
    let (links, _) = library::load(&ctx, &user, &query).await?;
    Ok(Render(PinnedTpl {
        shell: Shell::new::<LibraryDashboard>(&user, Section::Pinned),
        links,
    }))
}

#[derive(Debug, Template)]
#[template(path = "archive/unorganized.templ.html")]
pub(super) struct UnorganizedTpl {
    shell: Shell,
    links: Vec<LinkCard>,
}

pub(super) async fn unorganized(
    State(ctx): State<AppState>,
    user: AuthUser,
) -> Result<Render<UnorganizedTpl>> {
    let query = LinkQuery {
        unorganized_only: true,
        ..Default::default()
    };
    let (links, _) = library::load(&ctx, &user, &query).await?;
    Ok(Render(UnorganizedTpl {
        shell: Shell::new::<LibraryDashboard>(&user, Section::Unorganized),
        links,
    }))
}

#[derive(Debug, Template)]
#[template(path = "archive/link_detail.templ.html")]
pub(super) struct LinkDetailTpl {
    shell: Shell,
    link: LinkCard,
    latest_capture: Option<CaptureView>,
    highlights: Vec<HighlightView>,
    captures: Vec<CaptureView>,
    collections: Vec<CollectionChoice>,
    tags_csv: String,
    can_write: bool,
}

pub(super) async fn detail(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
) -> Result<Render<LinkDetailTpl>> {
    let detail = service::page::get(&ctx, &user, id).await?;
    let tags_csv = detail
        .page
        .tags
        .iter()
        .map(|tag| tag.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let selected = detail.page.record.collection_id.map(CollectionId::get);
    let highlights = service::page::highlights(&ctx, &user, id)
        .await?
        .into_iter()
        .map(HighlightView::from)
        .collect();
    let captures = service::page::captures(&ctx, &user, id, None, Some(Limit::from_const(50)))
        .await?
        .items
        .into_iter()
        .map(CaptureView::from)
        .collect::<Vec<_>>();

    Ok(Render(LinkDetailTpl {
        shell: Shell::new::<LibraryDashboard>(&user, Section::Links),
        latest_capture: detail.latest_capture.map(CaptureView::from),
        link: LinkCard::new(detail.page),
        collections: library::collection_choices(&ctx, &user, selected).await?,
        highlights,
        captures,
        tags_csv,
        can_write: user.can(Permission::ARCHIVE_WRITE),
    }))
}

pub(super) async fn create(
    State(ctx): State<AppState>,
    user: AuthUser,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<Fragments> {
    let upload = parse_page_upload(multipart?).await?;
    service::page::create(&ctx, &user, upload).await?;
    Fragments::new().toast(ToastKind::Success, "Link saved")
}

/// Multipart edit form. Absent text fields fall back to the current values.
pub(super) async fn update(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
    _headers: HeaderMap,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<Fragments> {
    let mut mp = multipart?;

    let mut title: Option<String> = None;
    let mut description: Option<String> = None;
    let mut collection: Option<String> = None;
    let mut tags: Option<String> = None;
    let mut pinned: Option<String> = None;
    let mut thumbnail_bytes: Option<Vec<u8>> = None;

    while let Some(field) = mp.next_field().await? {
        let name = field.name().unwrap_or("").to_owned();
        match name.as_str() {
            "displayTitle" => title = Some(field.text().await?),
            "description" => description = Some(field.text().await?),
            "collectionId" => collection = Some(field.text().await?),
            "tags" => tags = Some(field.text().await?),
            "pinned" => pinned = Some(field.text().await?),
            "thumbnail" => {
                let bytes = field.bytes().await?.to_vec();
                thumbnail_bytes = (!bytes.is_empty()).then_some(bytes);
            }
            _ => {}
        }
    }

    let patch = UpdatePage {
        display_title: title.map_or(FieldPatch::Unchanged, FieldPatch::Set),
        description: description.map_or(FieldPatch::Unchanged, FieldPatch::Set),
        collection_id: collection.map_or(FieldPatch::Unchanged, |value| {
            if value.trim().is_empty() {
                FieldPatch::Clear
            } else {
                FieldPatch::Set(value)
            }
        }),
        pinned: match pinned {
            Some(value) => FieldPatch::Set(parse_bool(&value)?),
            None => FieldPatch::Unchanged,
        },
        tags: tags.map_or(FieldPatch::Unchanged, FieldPatch::Set),
    };
    service::page::patch(&ctx, &user, id, patch).await?;

    if let Some(bytes) = thumbnail_bytes {
        let format = ImageFormat::detect(&bytes).ok_or_else(|| {
            AppError::invalid("invalid_image_format", "cannot determine thumbnail format")
        })?;
        service::page::set_thumbnail(&ctx, &user, id, Some(Image::new(format, bytes)?)).await?;
    }

    Fragments::new().toast(ToastKind::Success, "Link updated")
}

pub(super) async fn remove(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
) -> Result<Fragments> {
    service::page::delete(&ctx, &user, id).await?;
    Fragments::new().toast(ToastKind::Success, "Link deleted")
}

pub(super) async fn pin(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
) -> Result<Fragments> {
    let pinned = service::page::toggle_pin(&ctx, &user, id).await?;
    let message = if pinned { "Pinned" } else { "Unpinned" };
    Fragments::new().toast(ToastKind::Success, message)
}

#[derive(Debug, Deserialize)]
pub(super) struct HighlightForm {
    pub text: String,
    pub color: Option<String>,
}

pub(super) async fn add_highlight(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
    AppForm(form): AppForm<HighlightForm>,
) -> Result<Fragments> {
    service::page::add_highlight(&ctx, &user, id, &form.text, form.color.as_deref()).await?;
    Fragments::new().toast(ToastKind::Success, "Highlight saved")
}

pub(super) async fn update_highlight(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath((page_id, highlight_id)): AppPath<(PageId, i64)>,
    AppForm(form): AppForm<HighlightForm>,
) -> Result<Fragments> {
    let _ = page_id;
    service::page::update_highlight(
        &ctx,
        &user,
        highlight_id,
        Some(&form.text),
        form.color.as_deref(),
    )
    .await?;
    Fragments::new().toast(ToastKind::Success, "Highlight updated")
}

pub(super) async fn delete_highlight(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath((_page_id, highlight_id)): AppPath<(PageId, i64)>,
) -> Result<Fragments> {
    service::page::delete_highlight(&ctx, &user, highlight_id).await?;
    Fragments::new().toast(ToastKind::Success, "Highlight removed")
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/links", get(index).post(create))
        .route("/links/search", get(search))
        .route("/links/pinned", get(pinned))
        .route("/links/unorganized", get(unorganized))
        .route("/links/{id}", get(detail).put(update).delete(remove))
        .route("/links/{id}/pin", post(pin))
        .route("/links/{id}/highlights", post(add_highlight))
        .route(
            "/links/{id}/highlights/{highlight_id}",
            put(update_highlight).delete(delete_highlight),
        )
        .route("/links/{id}/archive", get(redirect_latest_archive))
}

async fn redirect_latest_archive(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
) -> Result<axum::response::Response> {
    let capture =
        service::capture::latest_with_blob(&ctx, &user, id, crate::db::capture::CaptureBlob::Html)
            .await?;
    Ok(
        axum::response::Redirect::temporary(&format!("/api/capture/{capture}/html"))
            .into_response(),
    )
}

use axum::response::IntoResponse as _;
