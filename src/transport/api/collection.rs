//! Collections: flat, per-owner folders that group links.

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, put},
};
use serde::Deserialize;

use crate::{
    app::AppState,
    db::collection::{CollectionId, CollectionRecord},
    error::Result,
    identity::AuthUser,
    service,
    transport::extract::body::{AppJson, AppPath},
};

#[derive(Debug, Deserialize)]
struct CollectionRequest {
    name: String,
    emoji: Option<String>,
    description: Option<String>,
    color: Option<String>,
    /// Accepted for wire compatibility; collections are flat in this schema.
    #[serde(default)]
    #[expect(dead_code, reason = "accepted but ignored: collections are flat")]
    parent_id: Option<String>,
}

impl CollectionRequest {
    fn fields(&self, public: bool) -> service::collection::CollectionFields {
        service::collection::CollectionFields {
            name: self.name.clone(),
            emoji: self.emoji.clone().unwrap_or_default(),
            description: self.description.clone().unwrap_or_default(),
            color: self.color.clone().unwrap_or_default(),
            public,
        }
    }
}

async fn list_collections(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<AppJson<Vec<CollectionRecord>>> {
    Ok(AppJson(service::collection::list(&state.db, &user).await?))
}

async fn get_collection(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
) -> Result<AppJson<CollectionRecord>> {
    Ok(AppJson(
        service::collection::get(&state.db, &user, id).await?,
    ))
}

async fn create_collection(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<CollectionRequest>,
) -> Result<AppJson<CollectionRecord>> {
    Ok(AppJson(
        service::collection::create(&state.db, &user, &req.fields(false)).await?,
    ))
}

async fn update_collection(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
    AppJson(req): AppJson<CollectionRequest>,
) -> Result<AppJson<CollectionRecord>> {
    let current = service::collection::get(&state.db, &user, id).await?;
    Ok(AppJson(
        service::collection::update(&state.db, &user, id, &req.fields(current.public)).await?,
    ))
}

async fn delete_collection(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
) -> Result<StatusCode> {
    service::collection::delete(&state.db, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct SharingRequest {
    public: Option<bool>,
}

async fn update_sharing(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
    AppJson(req): AppJson<SharingRequest>,
) -> Result<StatusCode> {
    service::collection::set_public(&state.db, &user, id, req.public.unwrap_or(false)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn public_collection(
    State(state): State<AppState>,
    AppPath(id): AppPath<CollectionId>,
) -> Result<AppJson<CollectionRecord>> {
    Ok(AppJson(service::collection::public(&state.db, id).await?))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/collection", get(list_collections).post(create_collection))
        .route(
            "/collection/{id}",
            get(get_collection)
                .post(update_collection)
                .put(update_collection)
                .delete(delete_collection),
        )
        .route("/collection/{id}/sharing", put(update_sharing))
        .route("/public/collection/{id}", get(public_collection))
}
