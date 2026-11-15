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
    domain::{TagId, TagName},
    error::{AppError, OptionNotFound as _, Resource, Result},
    identity::{AuthUser, Permission},
    service,
    transport::{
        extract::body::{AppForm, AppPath, AppQuery},
        htmx::{Fragments, Render, ToastKind},
        web::{
            dashboard::LibraryDashboard,
            library,
            viewmodels::archive::TagRow,
            viewmodels::{Section, Shell},
        },
    },
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

async fn rows(
    state: &AppState,
    user: &crate::identity::AuthUser,
    query: &str,
) -> Result<Vec<TagRow>> {
    let wanted = TagName::normalize(query);
    let tags = service::tag::list(&state.db, user).await?;
    let rows = tags
        .into_iter()
        .filter(|tag| wanted.is_empty() || tag.name.as_str().contains(&wanted))
        .map(TagRow::from)
        .collect();
    Ok(rows)
}

pub(super) async fn index(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Render<TagsTpl>> {
    Ok(Render(TagsTpl {
        shell: Shell::new::<LibraryDashboard>(&user, Section::Tags),
        tags: rows(&state, &user, "").await?,
        can_write: user.can(Permission::ARCHIVE_WRITE),
    }))
}

pub(super) async fn search(
    State(state): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<TagQuery>,
) -> Result<Render<TagListTpl>> {
    Ok(Render(TagListTpl {
        tags: rows(&state, &user, &query.q).await?,
    }))
}

#[derive(Debug, Template)]
#[template(path = "archive/tag.templ.html")]
pub(super) struct TagTpl {
    shell: Shell,
    tag: TagRow,
    links: Vec<crate::transport::web::viewmodels::archive::LinkCard>,
}

pub(super) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(name): AppPath<String>,
) -> Result<Render<TagTpl>> {
    let tag = service::tag::find(&state.db, &user, &name)
        .await?
        .or_not_found(Resource::Tag)?;
    let links = library::load_tagged(&state, &user, tag.id, PAGE_LINKS).await?;
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
    State(state): State<AppState>,
    user: AuthUser,
    AppForm(form): AppForm<CreateTagForm>,
) -> Result<Fragments> {
    service::tag::create(&state.db, &user, &form.name, form.color.as_deref()).await?;
    Fragments::new().toast(ToastKind::Success, "Tag added")
}

#[derive(Debug, Deserialize)]
pub(super) struct RenameTagForm {
    pub name: String,
}

pub(super) async fn rename(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(name): AppPath<String>,
    AppForm(form): AppForm<RenameTagForm>,
) -> Result<Fragments> {
    let id = resolve_id(&state, &user, &name).await?;
    service::tag::patch(
        &state.db,
        &user,
        id,
        Some(&form.name),
        crate::db::patch::FieldPatch::Unchanged,
    )
    .await?;
    Fragments::new().toast(ToastKind::Success, "Tag renamed")
}

pub(super) async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(name): AppPath<String>,
) -> Result<Fragments> {
    let id = resolve_id(&state, &user, &name).await?;
    service::tag::delete(&state.db, &user, id).await?;
    Fragments::new().toast(ToastKind::Success, "Tag deleted")
}

/// The web routes are keyed by tag name; actions need the numeric id.
async fn resolve_id(
    state: &AppState,
    user: &crate::identity::AuthUser,
    name: &str,
) -> Result<TagId, AppError> {
    service::tag::find(&state.db, user, name)
        .await?
        .map(|tag| tag.id)
        .or_not_found(Resource::Tag)
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/tags", get(index).post(create))
        .route("/tags/search", get(search))
        .route("/tags/{tag}", get(show))
        .route("/tags/{tag}", patch(rename).delete(remove))
}
