use axum::{
    Router,
    extract::{Path, State},
    routing::{get, put},
};
use http::StatusCode;
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    AppState,
    db::collection::{CollectionId, CollectionInput, CollectionRecord, non_empty_parent},
    error::{AppError, OptionExt, Result},
    extract::ApiJson,
};

async fn list_collection(State(pool): State<SqlitePool>) -> Result<ApiJson<Vec<CollectionRecord>>> {
    let collections = CollectionRecord::list(&pool).await?;
    Ok(ApiJson(collections))
}

async fn get_collection(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
) -> Result<ApiJson<CollectionRecord>> {
    let record = CollectionRecord::load(&pool, id)
        .await?
        .or_not_found("collection")?;

    Ok(ApiJson(record))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CollectionCreateRequest {
    name: String,
    emoji: Option<String>,
    description: Option<String>,
    parent_id: Option<String>,
    color: Option<String>,
}

impl CollectionCreateRequest {
    fn map_collection_input(&self, parent_id: Option<CollectionId>) -> CollectionInput<'_> {
        let emoji = self
            .emoji
            .as_deref()
            .map(str::trim)
            .filter(|emoji| !emoji.is_empty())
            .unwrap_or("📁");

        CollectionInput {
            name: self.name.trim(),
            emoji,
            description: self.description.as_deref().unwrap_or_default(),
            color: self.color.as_deref().unwrap_or("#6d5dfc"),
            parent_id,
            public: false,
        }
    }

    fn map_collection_update_input(
        &self,
        parent_id: Option<CollectionId>,
        public: bool,
    ) -> CollectionInput<'_> {
        CollectionInput {
            public,
            ..self.map_collection_input(parent_id)
        }
    }
}

async fn create_collection(
    State(pool): State<SqlitePool>,
    ApiJson(req): ApiJson<CollectionCreateRequest>,
) -> Result<ApiJson<CollectionRecord>> {
    if req.name.trim().is_empty() {
        return Err(AppError::invalid(
            "missing_name",
            "A collection needs a name.",
        ));
    }

    let parent_id = CollectionId::parse_or(req.parent_id.as_ref());
    let parent_id = non_empty_parent(&pool, parent_id, None).await?;
    let collection = CollectionRecord::create(&pool, &req.map_collection_input(parent_id)).await?;

    Ok(ApiJson(collection))
}

async fn update_collection(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
    ApiJson(req): ApiJson<CollectionCreateRequest>,
) -> Result<ApiJson<CollectionRecord>> {
    if req.name.trim().is_empty() {
        return Err(AppError::invalid(
            "missing_name",
            "A collection needs a name.",
        ));
    }

    let current = CollectionRecord::load(&pool, id)
        .await?
        .or_not_found("collection")?;
    let parent_id = CollectionId::parse_or(req.parent_id.as_ref());
    let parent_id = non_empty_parent(&pool, parent_id, Some(id)).await?;

    let Some(updated) = CollectionRecord::update(
        &pool,
        id,
        &req.map_collection_update_input(parent_id, current.public),
    )
    .await?
    else {
        return Err(AppError::NotFound("collection"));
    };

    Ok(ApiJson(updated))
}

async fn delete_collection(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
) -> Result<StatusCode> {
    if CollectionRecord::delete(&pool, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound("collection"))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharingForm {
    public: Option<bool>,
}

async fn update_sharing(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
    ApiJson(req): ApiJson<SharingForm>,
) -> Result<StatusCode> {
    if CollectionRecord::set_public(&pool, id, req.public.unwrap_or(false)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound("collection"))
    }
}

pub(crate) async fn public_collection(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
) -> Result<ApiJson<CollectionRecord>> {
    let record = CollectionRecord::load(&pool, id)
        .await?
        .or_not_found("collection")?;
    if !record.public {
        return Err(AppError::NotFound("public collection"));
    }

    Ok(ApiJson(record))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/collection", get(list_collection).post(create_collection))
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
