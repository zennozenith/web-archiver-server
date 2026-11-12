//! Administrator use-cases: users, roles, audit log.
//! Every function starts with the permission check, so a new adapter
//! (web UI, CLI) can't forget it.

use serde_json::json;

use crate::{
    app::AppState,
    auth::{
        AccountStatus, AuditEntry, AuthUser, Permission, RoleView, UserFilter, UserView, policy,
    },
    db::{self, audit::audit},
    error::{AppError, Result},
    util::time::now_ms,
};

const AUDIT_DEFAULT_LIMIT: i64 = 100;
const AUDIT_MAX_LIMIT: i64 = 500;

async fn target_id(ctx: &AppState, public_id: &str) -> Result<i64> {
    db::user::find_id_by_public_id(&ctx.db, public_id)
        .await?
        .ok_or(AppError::NotFound("user"))
}

// ----------------------------------------------------------------- users

pub(crate) async fn list_users(
    ctx: &AppState,
    actor: &AuthUser,
    filter: &UserFilter,
) -> Result<Vec<UserView>> {
    actor.require(Permission::USERS_READ)?;
    Ok(db::user::list(&ctx.db, filter).await?)
}

pub(crate) async fn set_status(
    ctx: &AppState,
    actor: &AuthUser,
    public_id: &str,
    status: AccountStatus,
    reason: Option<&str>,
) -> Result<()> {
    actor.require(Permission::USERS_WRITE)?;
    let target = target_id(ctx, public_id).await?;
    policy::check_not_self(actor.id, target, "you cannot change your own status")?;

    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    if !db::user::set_status(&ctx.db, target, status, reason).await? {
        return Err(AppError::NotFound("user"));
    }

    audit(
        &ctx.db,
        Some(actor.id),
        Some(target),
        "user.status_changed",
        Some(json!({ "status": status, "reason": reason })),
    )
    .await;
    Ok(())
}

pub(crate) async fn delete_user(ctx: &AppState, actor: &AuthUser, public_id: &str) -> Result<()> {
    actor.require(Permission::USERS_DELETE)?;
    let target = target_id(ctx, public_id).await?;
    policy::check_not_self(actor.id, target, "you cannot delete yourself")?;

    if !db::user::soft_delete(&ctx.db, target, now_ms()).await? {
        return Err(AppError::NotFound("user"));
    }
    audit(&ctx.db, Some(actor.id), Some(target), "user.deleted", None).await;
    Ok(())
}

// ----------------------------------------------------------------- roles

pub(crate) async fn list_roles(ctx: &AppState, actor: &AuthUser) -> Result<Vec<RoleView>> {
    actor.require(Permission::ROLES_READ)?;
    Ok(db::role::list(&ctx.db).await?)
}

pub(crate) async fn grant_role(
    ctx: &AppState,
    actor: &AuthUser,
    public_id: &str,
    role: &str,
) -> Result<()> {
    actor.require(Permission::ROLES_WRITE)?;
    policy::check_role_assignable(actor, role)?;

    let target = target_id(ctx, public_id).await?;
    let role_id = db::role::find_id(&ctx.db, role)
        .await?
        .ok_or(AppError::NotFound("role"))?;

    // Granting twice is a no-op, and not worth an audit row.
    if db::role::grant(&ctx.db, target, role_id, actor.id).await? {
        audit(
            &ctx.db,
            Some(actor.id),
            Some(target),
            "role.granted",
            Some(json!({ "role": role })),
        )
        .await;
    }
    Ok(())
}

pub(crate) async fn revoke_role(
    ctx: &AppState,
    actor: &AuthUser,
    public_id: &str,
    role: &str,
) -> Result<()> {
    actor.require(Permission::ROLES_WRITE)?;
    policy::check_role_assignable(actor, role)?;

    let target = target_id(ctx, public_id).await?;
    if !db::role::revoke(&ctx.db, target, role).await? {
        return Err(AppError::NotFound("user_or_role"));
    }
    audit(
        &ctx.db,
        Some(actor.id),
        Some(target),
        "role.revoked",
        Some(json!({ "role": role })),
    )
    .await;
    Ok(())
}

// ----------------------------------------------------------------- audit

pub(crate) async fn list_audit(
    ctx: &AppState,
    actor: &AuthUser,
    limit: Option<i64>,
) -> Result<Vec<AuditEntry>> {
    actor.require(Permission::AUDIT_READ)?;
    let limit = limit
        .unwrap_or(AUDIT_DEFAULT_LIMIT)
        .clamp(1, AUDIT_MAX_LIMIT);
    db::audit::list(&ctx.db, limit)
        .await
        .map_err(AppError::from)
}
