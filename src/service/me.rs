//! Self-service use-cases: profile, own sessions, own API keys.

use secrecy::ExposeSecret as _;
use serde_json::json;

use crate::{
    db::{
        self, Database,
        api_key::InsertApiKey,
        audit::{AuditAction, audit},
        permission::PermissionInfo,
    },
    error::{AppError, ForbiddenReason, Resource, Result},
    identity::{
        ApiKeyToken, ApiKeyView, AuthUser, CreatedApiKey, NewApiKey, Permission, ProfileUpdate,
        ProfileView, SessionView, UserView,
    },
    infra::sha256_hex,
    util::time::now_ms,
};

pub(crate) struct Mine {
    pub user: UserView,
    pub sessions: usize,
    pub api_keys: usize,
}

/// Permissions the actor holds, i.e. the scopes they may put on a new key.
pub(crate) async fn available_scopes(
    pool: &Database,
    actor: &AuthUser,
) -> Result<Vec<PermissionInfo>> {
    Ok(db::permission::all(pool.reader())
        .await?
        .into_iter()
        .filter(|p| actor.has_scope(&p.name))
        .collect())
}

pub(crate) struct Me {
    pub mine: Mine,
    pub profile: ProfileView,
}

pub(crate) async fn get_me(pool: &Database, actor: &AuthUser) -> Result<Me> {
    actor.require(Permission::PROFILE_READ)?;

    let read = pool.reader();
    let user = db::user::find(read, actor.id).await?;
    let profile = db::profile::find(read, actor.id).await?;
    let sessions = db::session::list_for_user(read, actor.id).await?;
    let keys = db::api_key::list_for_user(read, actor.id).await?;

    let mine = Mine {
        user,
        sessions: sessions.len(),
        api_keys: keys.len(),
    };

    Ok(Me { mine, profile })
}

pub(crate) async fn update_profile(
    pool: &Database,
    actor: &AuthUser,
    update: ProfileUpdate,
) -> Result<()> {
    actor.require(Permission::PROFILE_WRITE)?;
    let update = update.normalize()?;

    db::profile::update(pool.writer(), actor.id, &update).await?;
    audit(
        pool.writer(),
        Some(actor.id),
        Some(actor.id),
        AuditAction::ProfileUpdated,
        None,
    )
    .await;
    Ok(())
}

// -------------------------------------------------------------- sessions

pub(crate) async fn list_sessions(pool: &Database, actor: &AuthUser) -> Result<Vec<SessionView>> {
    let mut sessions = db::session::list_for_user(pool.reader(), actor.id).await?;
    for s in &mut sessions {
        s.current = actor.session_public_id.as_deref() == Some(s.public_id.as_str());
    }
    Ok(sessions)
}

pub(crate) async fn revoke_session(
    pool: &Database,
    actor: &AuthUser,
    public_id: &str,
) -> Result<()> {
    // Users can only end their own sessions; there is no admin session endpoint yet.
    if !db::session::revoke(pool.writer(), actor.id, public_id, now_ms()).await? {
        return Err(AppError::NotFound(Resource::Session));
    }
    audit(
        pool.writer(),
        Some(actor.id),
        Some(actor.id),
        AuditAction::SessionRevoked,
        Some(json!({ "session": public_id })),
    )
    .await;
    Ok(())
}

// -------------------------------------------------------------- API keys

pub(crate) async fn list_api_keys(pool: &Database, actor: &AuthUser) -> Result<Vec<ApiKeyView>> {
    Ok(db::api_key::list_for_user(pool.reader(), actor.id).await?)
}

pub(crate) async fn create_api_key(
    pool: &Database,
    actor: &AuthUser,
    req: NewApiKey,
) -> Result<CreatedApiKey> {
    if actor.via_api_key {
        // keys cannot mint more keys
        return Err(AppError::Forbidden(ForbiddenReason::ApiKeyMint));
    }
    // A key may only carry scopes its owner actually has.
    if req.scopes().iter().any(|s| !actor.has_scope(s)) {
        return Err(AppError::Forbidden(ForbiddenReason::MissingPermission));
    }

    let key = ApiKeyToken::new()?;
    let prefix: String = key.expose_secret().chars().take(8).collect();
    let scopes_json = req.scopes_json()?;

    let public_id = db::api_key::insert(
        pool.writer(),
        InsertApiKey {
            user_id: actor.id,
            name: req.name(),
            prefix: &prefix,
            key_hash: &sha256_hex(key.expose_secret()),
            scopes_json: &scopes_json,
            expires_at: req.expires_at(now_ms()),
        },
    )
    .await
    .map_err(|e| AppError::unique(e, "that API key already exists"))?;

    audit(
        pool.writer(),
        Some(actor.id),
        Some(actor.id),
        AuditAction::ApiKeyCreated,
        Some(json!({ "name": req.name() })),
    )
    .await;

    Ok(CreatedApiKey { public_id, key })
}

pub(crate) async fn revoke_api_key(
    pool: &Database,
    actor: &AuthUser,
    public_id: &str,
) -> Result<()> {
    if !db::api_key::revoke(pool.writer(), actor.id, public_id, now_ms()).await? {
        return Err(AppError::NotFound(Resource::ApiKey));
    }
    audit(
        pool.writer(),
        Some(actor.id),
        Some(actor.id),
        AuditAction::ApiKeyRevoked,
        Some(json!({ "key": public_id })),
    )
    .await;
    Ok(())
}
