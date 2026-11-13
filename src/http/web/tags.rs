//! Tag pages: listing, search fragment, detail and actions.

use askama::Template;
use axum::{
    Router,
    extract::State,
    routing::{get, patch},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    auth::Permission,
    error::{AppError, Result},
    http::{
        extract::{
            body::{AppPath, AppQuery},
            web_user::WebUser,
        },
        htmx::{ActionError, Fragments, PageError, Render, ToastKind},
        web::{
            dashboard::LibraryDashboard,
            form::ActionForm,
            library,
            view::{Section, Shell},
            view_archive::TagRow,
        },
    },
    infra::{TagId, TagName},
    service,
    util::limit::Limit,
};

const PAGE_LINKS: Limit = Limit::from_const(200);

#[derive(Debug, Default, Deserialize)]
pub(super) struct TagQuery {
    #[serde(default)]
    q: String,
}

#[derive(Debug, Template)]
#[template(path = "archive/_tag_list.templ.html")]
pub(super) struct TagListTpl {
    tags: Vec<TagRow>,
}

#[derive(Debug, Template)]
#[template(path = "archive/tags.templ.html")]
pub(super) struct TagsTpl {
    shell: Shell,
    tags: Vec<TagRow>,
    can_write: bool,
}

async fn rows(ctx: &AppState, user: &crate::auth::AuthUser, query: &str) -> Result<Vec<TagRow>> {
    let wanted = TagName::normalize(query);
    let tags = service::tag::list(ctx, user).await?;
    let rows = tags
        .into_iter()
        .filter(|tag| wanted.is_empty() || tag.name.as_str().contains(&wanted))
        .map(TagRow::from)
        .collect();
    Ok(rows)
}

pub(super) async fn index(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
) -> Result<Render<TagsTpl>, PageError> {
    Ok(Render(TagsTpl {
        shell: Shell::new::<LibraryDashboard>(&user, Section::Tags),
        tags: rows(&ctx, &user, "").await?,
        can_write: user.can(Permission::ARCHIVE_WRITE),
    }))
}

pub(super) async fn search(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
    AppQuery(query): AppQuery<TagQuery>,
) -> Result<Render<TagListTpl>, ActionError> {
    Ok(Render(TagListTpl {
        tags: rows(&ctx, &user, &query.q).await?,
    }))
}

#[derive(Debug, Template)]
#[template(path = "archive/tag.templ.html")]
pub(super) struct TagTpl {
    shell: Shell,
    tag: TagRow,
    links: Vec<crate::http::web::view_archive::LinkCard>,
}

pub(super) async fn show(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
    AppPath(name): AppPath<String>,
) -> Result<Render<TagTpl>, PageError> {
    let tag = service::tag::find(&ctx, &user, &name)
        .await?
        .ok_or(AppError::NotFound("tag"))?;
    let links = library::load_tagged(&ctx, &user, tag.id, PAGE_LINKS).await?;
    Ok(Render(TagTpl {
        shell: Shell::new::<LibraryDashboard>(&user, Section::Tags),
        tag: TagRow::from(tag),
        links,
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateTagForm {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

pub(super) async fn create(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
    ActionForm(form): ActionForm<CreateTagForm>,
) -> Result<Fragments, ActionError> {
    service::tag::create(&ctx, &user, &form.name, form.color.as_deref()).await?;
    Ok(Fragments::new().toast(ToastKind::Success, "Tag added")?)
}

#[derive(Debug, Deserialize)]
pub(super) struct RenameTagForm {
    pub name: String,
}

pub(super) async fn rename(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
    AppPath(name): AppPath<String>,
    ActionForm(form): ActionForm<RenameTagForm>,
) -> Result<Fragments, ActionError> {
    let id = resolve_id(&ctx, &user, &name).await?;
    service::tag::patch(
        &ctx,
        &user,
        id,
        Some(&form.name),
        crate::db::helper::FieldPatch::Unchanged,
    )
    .await?;
    Ok(Fragments::new().toast(ToastKind::Success, "Tag renamed")?)
}

pub(super) async fn remove(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
    AppPath(name): AppPath<String>,
) -> Result<Fragments, ActionError> {
    let id = resolve_id(&ctx, &user, &name).await?;
    service::tag::delete(&ctx, &user, id).await?;
    Ok(Fragments::new().toast(ToastKind::Success, "Tag deleted")?)
}

/// The web routes are keyed by tag name; actions need the numeric id.
async fn resolve_id(
    ctx: &AppState,
    user: &crate::auth::AuthUser,
    name: &str,
) -> Result<TagId, AppError> {
    service::tag::find(ctx, user, name)
        .await?
        .map(|tag| tag.id)
        .ok_or(AppError::NotFound("tag"))
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/tags", get(index).post(create))
        .route("/tags/search", get(search))
        .route("/tags/{tag}", get(show))
        .route("/tags/{tag}", patch(rename).delete(remove))
}
