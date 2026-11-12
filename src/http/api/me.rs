use axum::{
    Json, Router,
    extract::State,
    routing::{delete, get, put},
};
use serde_json::{Value, json};

use crate::{
    app::AppState,
    auth::{AuthUser, NewApiKey},
    error::Result,
    http::{
        dto::{
            OkDto,
            user::{
                ApiKeyDto, CreateKeyRequest, CreatedApiKeyDto, MeDto, ProfileRequest, SessionDto,
            },
        },
        extract::body::{AppJson, AppPath},
    },
    service,
};

async fn get_me(State(ctx): State<AppState>, user: AuthUser) -> Result<Json<MeDto>> {
    let me = service::me::get_me(&ctx, &user).await?;
    Ok(Json(me.into()))
}

async fn update_profile(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<ProfileRequest>,
) -> Result<Json<OkDto>> {
    service::me::update_profile(&ctx, &user, req.into()).await?;
    Ok(Json(OkDto::new()))
}

async fn list_sessions(State(ctx): State<AppState>, user: AuthUser) -> Result<Json<Value>> {
    let sessions: Vec<SessionDto> = service::me::list_sessions(&ctx, &user)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(json!({ "sessions": sessions })))
}

async fn revoke_session(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(public_id): AppPath<String>,
) -> Result<Json<OkDto>> {
    service::me::revoke_session(&ctx, &user, &public_id).await?;
    Ok(Json(OkDto::new()))
}

async fn list_api_keys(State(ctx): State<AppState>, user: AuthUser) -> Result<Json<Value>> {
    let keys: Vec<ApiKeyDto> = service::me::list_api_keys(&ctx, &user)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(json!({ "api_keys": keys })))
}

async fn create_api_key(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<CreateKeyRequest>,
) -> Result<Json<CreatedApiKeyDto>> {
    let new_key = NewApiKey::try_from(req)?;
    let created = service::me::create_api_key(&ctx, &user, new_key).await?;
    // The plaintext key is shown exactly once.
    Ok(Json(created.into()))
}

async fn revoke_api_key(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(public_id): AppPath<String>,
) -> Result<Json<OkDto>> {
    service::me::revoke_api_key(&ctx, &user, &public_id).await?;
    Ok(Json(OkDto::new()))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/me", get(get_me))
        .route("/me/profile", put(update_profile))
        .route("/me/sessions", get(list_sessions))
        .route("/me/sessions/{public_id}", delete(revoke_session))
        .route("/me/api-keys", get(list_api_keys).post(create_api_key))
        .route("/me/api-keys/{public_id}", delete(revoke_api_key))
}
