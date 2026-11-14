//! Collection pages: list, detail, create/edit/delete and public sharing.

use askama::Template;
use axum::{
    Router,
    extract::State,
    routing::{get, put},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    db::collection::CollectionId,
    error::Result,
    identity::{AuthUser, Permission},
    service,
    transport::{
        extract::body::{AppForm, AppPath},
        htmx::{Fragments, Render, ToastKind},
        web::{
            dashboard::LibraryDashboard,
            library,
            viewmodels::archive::{CollectionCard, LinkCard},
            viewmodels::{Section, Shell},
        },
    },
    util::limit::Limit,
};

const PAGE_LINKS: Limit = Limit::from_const(200);

#[derive(Debug, Deserialize)]
pub(super) struct CollectionForm {
    pub name: String,
    #[serde(default)]
    pub emoji: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub public: Option<String>,
}

impl CollectionForm {
    fn fields(&self) -> service::collection::CollectionFields {
        service::collection::CollectionFields {
            name: self.name.clone(),
            emoji: self.emoji.clone(),
            description: self.description.clone(),
            color: self.color.clone(),
            public: self
                .public
                .as_deref()
                .is_some_and(|v| v == "true" || v == "on"),
        }
    }
}

#[derive(Debug, Template)]
#[template(path = "archive/collections.templ.html")]
pub(super) struct CollectionsTpl {
    shell: Shell,
    collections: Vec<CollectionCard>,
    can_write: bool,
}

pub(super) async fn index(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Render<CollectionsTpl>> {
    let collections = service::collection::list(&state.db, &user)
        .await?
        .into_iter()
        .map(CollectionCard::from)
        .collect();
    Ok(Render(CollectionsTpl {
        shell: Shell::new::<LibraryDashboard>(&user, Section::Collections),
        collections,
        can_write: user.can(Permission::ARCHIVE_WRITE),
    }))
}

#[derive(Debug, Template)]
#[template(path = "archive/collection.templ.html")]
pub(super) struct CollectionTpl {
    shell: Shell,
    collection: CollectionCard,
    links: Vec<LinkCard>,
    can_write: bool,
}

pub(super) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
) -> Result<Render<CollectionTpl>> {
    let collection = service::collection::get(&state.db, &user, id).await?;
    let links = library::load_collection(&state, &user, id, PAGE_LINKS).await?;
    Ok(Render(CollectionTpl {
        shell: Shell::new::<LibraryDashboard>(&user, Section::Collections),
        collection: CollectionCard::from(collection),
        links,
        can_write: user.can(Permission::ARCHIVE_WRITE),
    }))
}

#[derive(Debug, Template)]
#[template(path = "archive/collection_form.templ.html")]
pub(super) struct CollectionFormTpl {
    shell: Shell,
    form_title: &'static str,
    form_action: String,
    method: &'static str,
    redirect_to: &'static str,
    submit_label: &'static str,
    name: String,
    emoji: String,
    description: String,
    color: String,
    public: bool,
}

fn form_shell(user: &crate::identity::AuthUser) -> Shell {
    Shell::new::<LibraryDashboard>(user, Section::Collections)
}

pub(super) async fn new_form(
    State(_ctx): State<AppState>,
    user: AuthUser,
) -> Result<Render<CollectionFormTpl>> {
    Ok(Render(CollectionFormTpl {
        shell: form_shell(&user),
        form_title: "New collection",
        form_action: "/collections".to_owned(),
        method: "post",
        redirect_to: "/collections",
        submit_label: "Create collection",
        name: String::new(),
        emoji: "📁".to_owned(),
        description: String::new(),
        color: "#6d5dfc".to_owned(),
        public: false,
    }))
}

pub(super) async fn edit_form(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
) -> Result<Render<CollectionFormTpl>> {
    let collection = service::collection::get(&state.db, &user, id).await?;
    Ok(Render(CollectionFormTpl {
        shell: form_shell(&user),
        form_title: "Edit collection",
        form_action: format!("/collections/{id}"),
        method: "put",
        redirect_to: "/collections",
        submit_label: "Save changes",
        name: collection.name,
        emoji: collection.emoji,
        description: collection.description,
        color: collection.color,
        public: collection.public,
    }))
}

pub(super) async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    AppForm(form): AppForm<CollectionForm>,
) -> Result<Fragments> {
    service::collection::create(&state.db, &user, &form.fields()).await?;
    Fragments::new().toast(ToastKind::Success, "Collection created")
}

pub(super) async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
    AppForm(form): AppForm<CollectionForm>,
) -> Result<Fragments> {
    service::collection::update(&state.db, &user, id, &form.fields()).await?;
    Fragments::new().toast(ToastKind::Success, "Collection updated")
}

pub(super) async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
) -> Result<Fragments> {
    service::collection::delete(&state.db, &user, id).await?;
    Fragments::new().toast(ToastKind::Success, "Collection deleted")
}

#[derive(Debug, Deserialize)]
pub(super) struct SharingForm {
    #[serde(default)]
    public: Option<String>,
}

pub(super) async fn sharing(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
    AppForm(form): AppForm<SharingForm>,
) -> Result<Fragments> {
    let public = form
        .public
        .as_deref()
        .is_some_and(|value| value == "true" || value == "on");
    service::collection::set_public(&state.db, &user, id, public).await?;
    let message = if public {
        "Collection shared"
    } else {
        "Collection made private"
    };
    Fragments::new().toast(ToastKind::Success, message)
}

#[derive(Debug, Template)]
#[template(path = "archive/public_collection.templ.html")]
pub(super) struct PublicCollectionTpl {
    shell: Shell,
    collection: CollectionCard,
    links: Vec<LinkCard>,
}

pub(super) async fn public(
    State(state): State<AppState>,
    AppPath(id): AppPath<CollectionId>,
) -> Result<Render<PublicCollectionTpl>> {
    let (collection, items) = service::collection::public_links(&state.db, id, PAGE_LINKS).await?;
    Ok(Render(PublicCollectionTpl {
        shell: Shell::public("Shared collection"),
        collection: CollectionCard::from(collection),
        links: library::cards(items),
    }))
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/collections", get(index).post(create))
        .route("/collections/new", get(new_form))
        .route("/collections/{id}", get(show).put(update).delete(remove))
        .route("/collections/{id}/edit", get(edit_form))
        .route("/collections/{id}/sharing", put(sharing))
}

pub(crate) fn public_router() -> Router<AppState> {
    Router::new().route("/shared/collection/{id}", get(public))
}
