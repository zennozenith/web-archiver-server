//! Self-service use-cases: profile, own sessions, own API keys.

use secrecy::ExposeSecret as _;
use serde_json::json;

use crate::{
    app::AppState,
    auth::{
        ApiKeyToken, ApiKeyView, AuthUser, CreatedApiKey, NewApiKey, Permission, ProfileUpdate,
        ProfileView, SessionView, UserView,
    },
    db::{self, api_key::InsertApiKey, audit::audit},
    error::{AppError, ForbiddenReason, Result},
    infra::sha256_hex,
    util::time::now_ms,
};

pub(crate) struct Me {
    pub user: UserView,
    pub profile: ProfileView,
    pub permissions: Vec<String>,
}

pub(crate) async fn get_me(ctx: &AppState, actor: &AuthUser) -> Result<Me> {
    actor.require(Permission::PROFILE_READ)?;

    let (user, profile) = tokio::try_join!(
        db::user::find(&ctx.db, actor.id),
        db::profile::find(&ctx.db, actor.id),
    )?;

    let mut permissions: Vec<String> = actor.permissions().map(str::to_owned).collect();
    permissions.sort_unstable();

    Ok(Me {
        user,
        profile,
        permissions,
    })
}

pub(crate) async fn update_profile(
    ctx: &AppState,
    actor: &AuthUser,
    update: ProfileUpdate,
) -> Result<()> {
    actor.require(Permission::PROFILE_WRITE)?;
    let update = update.normalize()?;

    db::profile::update(&ctx.db, actor.id, &update).await?;
    audit(
        &ctx.db,
        Some(actor.id),
        Some(actor.id),
        "profile.updated",
        None,
    )
    .await;
    Ok(())
}

// -------------------------------------------------------------- sessions

pub(crate) async fn list_sessions(ctx: &AppState, actor: &AuthUser) -> Result<Vec<SessionView>> {
    let mut sessions = db::session::list_for_user(&ctx.db, actor.id).await?;
    for s in &mut sessions {
        s.current = actor.session_public_id.as_deref() == Some(s.public_id.as_str());
    }
    Ok(sessions)
}

pub(crate) async fn revoke_session(
    ctx: &AppState,
    actor: &AuthUser,
    public_id: &str,
) -> Result<()> {
    // Users can only end their own sessions; there is no admin session endpoint yet.
    if !db::session::revoke(&ctx.db, actor.id, public_id, now_ms()).await? {
        return Err(AppError::NotFound("session"));
    }
    audit(
        &ctx.db,
        Some(actor.id),
        Some(actor.id),
        "session.revoked",
        Some(json!({ "session": public_id })),
    )
    .await;
    Ok(())
}

// -------------------------------------------------------------- API keys

pub(crate) async fn list_api_keys(ctx: &AppState, actor: &AuthUser) -> Result<Vec<ApiKeyView>> {
    Ok(db::api_key::list_for_user(&ctx.db, actor.id).await?)
}

pub(crate) async fn create_api_key(
    ctx: &AppState,
    actor: &AuthUser,
    req: NewApiKey,
) -> Result<CreatedApiKey> {
    if actor.via_api_key {
        // keys cannot mint more keys
        return Err(AppError::Forbidden(ForbiddenReason::ApiKeyMint));
    }
    // A key may only carry scopes its owner actually has.
    if req.scopes().iter().any(|s| !actor.has(s)) {
        return Err(AppError::Forbidden(ForbiddenReason::Permission));
    }

    let key = ApiKeyToken::new()?;
    let prefix: String = key.expose_secret().chars().take(8).collect();
    let scopes_json = req.scopes_json()?;

    let public_id = db::api_key::insert(
        &ctx.db,
        InsertApiKey {
            user_id: actor.id,
            name: req.name(),
            prefix: &prefix,
            key_hash: &sha256_hex(key.expose_secret()),
            scopes_json: &scopes_json,
            expires_at: req.expires_at(now_ms()),
        },
    )
    .await?;

    audit(
        &ctx.db,
        Some(actor.id),
        Some(actor.id),
        "api_key.created",
        Some(json!({ "name": req.name() })),
    )
    .await;

    Ok(CreatedApiKey { public_id, key })
}

pub(crate) async fn revoke_api_key(
    ctx: &AppState,
    actor: &AuthUser,
    public_id: &str,
) -> Result<()> {
    if !db::api_key::revoke(&ctx.db, actor.id, public_id, now_ms()).await? {
        return Err(AppError::NotFound("api_key"));
    }
    audit(
        &ctx.db,
        Some(actor.id),
        Some(actor.id),
        "api_key.revoked",
        Some(json!({ "key": public_id })),
    )
    .await;
    Ok(())
}
