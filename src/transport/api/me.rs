use axum::{
    Router,
    extract::State,
    routing::{delete, get, put},
};
use serde_json::{Value, json};

use crate::{
    app::AppState,
    error::Result,
    identity::{AuthUser, NewApiKey},
    service,
    transport::{
        dto::{
            OkDto,
            user::{
                ApiKeyDto, CreateKeyRequest, CreatedApiKeyDto, MeDto, ProfileRequest, SessionDto,
            },
        },
        extract::body::{AppJson, AppPath},
    },
};

async fn get_me(State(state): State<AppState>, user: AuthUser) -> Result<AppJson<MeDto>> {
    let me = service::me::get_me(&state.db, &user).await?;
    Ok(AppJson(me.into()))
}

async fn update_profile(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<ProfileRequest>,
) -> Result<AppJson<OkDto>> {
    service::me::update_profile(state.db.writer(), &user, req.into()).await?;
    Ok(AppJson(OkDto::new()))
}

async fn list_sessions(State(state): State<AppState>, user: AuthUser) -> Result<AppJson<Value>> {
    let sessions: Vec<SessionDto> = service::me::list_sessions(&state.db, &user)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(AppJson(json!({ "sessions": sessions })))
}

async fn revoke_session(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(public_id): AppPath<String>,
) -> Result<AppJson<OkDto>> {
    service::me::revoke_session(state.db.writer(), &user, &public_id).await?;
    Ok(AppJson(OkDto::new()))
}

async fn list_api_keys(State(state): State<AppState>, user: AuthUser) -> Result<AppJson<Value>> {
    let keys: Vec<ApiKeyDto> = service::me::list_api_keys(&state.db, &user)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(AppJson(json!({ "api_keys": keys })))
}

async fn create_api_key(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(req): AppJson<CreateKeyRequest>,
) -> Result<AppJson<CreatedApiKeyDto>> {
    let new_key = NewApiKey::try_from(req)?;
    let created = service::me::create_api_key(state.db.writer(), &user, new_key).await?;
    // The plaintext key is shown exactly once.
    Ok(AppJson(created.into()))
}

async fn revoke_api_key(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(public_id): AppPath<String>,
) -> Result<AppJson<OkDto>> {
    service::me::revoke_api_key(state.db.writer(), &user, &public_id).await?;
    Ok(AppJson(OkDto::new()))
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
