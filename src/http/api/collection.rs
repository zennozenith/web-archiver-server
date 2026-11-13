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
    auth::AuthUser,
    db::collection::{CollectionId, CollectionRecord},
    error::Result,
    http::extract::body::{AppJson, AppPath},
    service,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    State(ctx): State<AppState>,
    user: AuthUser,
) -> Result<AppJson<Vec<CollectionRecord>>> {
    Ok(AppJson(service::collection::list(&ctx, &user).await?))
}

async fn get_collection(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
) -> Result<AppJson<CollectionRecord>> {
    Ok(AppJson(service::collection::get(&ctx, &user, id).await?))
}

async fn create_collection(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<CollectionRequest>,
) -> Result<AppJson<CollectionRecord>> {
    Ok(AppJson(
        service::collection::create(&ctx, &user, &req.fields(false)).await?,
    ))
}

async fn update_collection(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
    AppJson(req): AppJson<CollectionRequest>,
) -> Result<AppJson<CollectionRecord>> {
    let current = service::collection::get(&ctx, &user, id).await?;
    Ok(AppJson(
        service::collection::update(&ctx, &user, id, &req.fields(current.public)).await?,
    ))
}

async fn delete_collection(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
) -> Result<StatusCode> {
    service::collection::delete(&ctx, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharingRequest {
    public: Option<bool>,
}

async fn update_sharing(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<CollectionId>,
    AppJson(req): AppJson<SharingRequest>,
) -> Result<StatusCode> {
    service::collection::set_public(&ctx, &user, id, req.public.unwrap_or(false)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn public_collection(
    State(ctx): State<AppState>,
    AppPath(id): AppPath<CollectionId>,
) -> Result<AppJson<CollectionRecord>> {
    Ok(AppJson(service::collection::public(&ctx, id).await?))
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
