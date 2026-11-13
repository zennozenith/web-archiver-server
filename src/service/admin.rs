//! Administrator use-cases: users, roles, audit log.
//! Every function starts with the permission check, so a new adapter
//! (web UI, CLI) can't forget it.

use serde_json::json;

use crate::{
    app::AppState,
    auth::{
        AccountStatus, AuditEntry, AuthUser, Permission, RoleView, UserFilter, UserView, policy,
    },
    db::{
        self,
        audit::audit,
        permission::PermissionInfo,
        role::RoleSummary,
        user::{UserExtra, UserSearch},
    },
    error::{AppError, Result},
    util::time::now_ms,
};

const AUDIT_DEFAULT_LIMIT: i64 = 100;
const AUDIT_MAX_LIMIT: i64 = 500;
const AUDIT_PAGE: usize = 50;

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

pub(crate) struct UserPage {
    pub rows: Vec<UserView>,
    pub total: i64,
    /// The page actually served, after clamping to the available range.
    pub page: i64,
}

pub(crate) async fn search_users(
    ctx: &AppState,
    actor: &AuthUser,
    search: &UserSearch<'_>,
    page: i64,
    per_page: i64,
) -> Result<UserPage> {
    actor.require(Permission::USERS_READ)?;
    let per_page = per_page.clamp(1, 100);
    let total = db::user::count(&ctx.db, search).await?;
    let pages = total
        .saturating_add(per_page.saturating_sub(1))
        .checked_div(per_page)
        .unwrap_or(1)
        .max(1);
    let page = page.clamp(1, pages);
    let offset = page.saturating_sub(1).saturating_mul(per_page);
    let rows = db::user::search(&ctx.db, search, per_page, offset).await?;
    Ok(UserPage { rows, total, page })
}

pub(crate) struct RoleChoice {
    pub name: String,
    pub description: Option<String>,
    pub assigned: bool,
    /// False for the actor's own roles and roles above the actor's level.
    pub assignable: bool,
}

pub(crate) struct UserDetail {
    pub user: UserView,
    pub extra: UserExtra,
    pub roles: Vec<RoleChoice>,
    /// Empty unless the actor may read the audit log.
    pub activity: Vec<AuditEntry>,
}

pub(crate) async fn user_detail(
    ctx: &AppState,
    actor: &AuthUser,
    public_id: &str,
) -> Result<UserDetail> {
    actor.require(Permission::USERS_READ)?;
    let id = target_id(ctx, public_id).await?;

    let (user, extra) =
        tokio::try_join!(db::user::find(&ctx.db, id), db::user::extra(&ctx.db, id),)?;

    let assigned = user.roles();
    let is_self = actor.id == id;
    let roles = db::role::list(&ctx.db)
        .await?
        .into_iter()
        .map(|r| RoleChoice {
            assigned: assigned.contains(&r.name),
            assignable: !is_self && policy::check_role_assignable(actor, &r.name).is_ok(),
            description: r.description,
            name: r.name,
        })
        .collect();

    let activity = if actor.can(Permission::AUDIT_READ) {
        db::audit::for_user(&ctx.db, id, 5).await?
    } else {
        Vec::new()
    };
    Ok(UserDetail {
        user,
        extra,
        roles,
        activity,
    })
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

/// Role names for filter dropdowns. Needs only `users:read`.
pub(crate) async fn role_names(ctx: &AppState, actor: &AuthUser) -> Result<Vec<String>> {
    actor.require(Permission::USERS_READ)?;
    Ok(db::role::list(&ctx.db)
        .await?
        .into_iter()
        .map(|r| r.name)
        .collect())
}

pub(crate) struct RoleCatalog {
    pub roles: Vec<RoleSummary>,
    pub permissions: Vec<PermissionInfo>,
}

pub(crate) async fn role_catalog(ctx: &AppState, actor: &AuthUser) -> Result<RoleCatalog> {
    actor.require(Permission::ROLES_READ)?;
    let (roles, permissions) = tokio::try_join!(
        db::role::summaries(&ctx.db, now_ms()),
        db::permission::all(&ctx.db),
    )?;
    Ok(RoleCatalog { roles, permissions })
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

pub(crate) struct AuditPage {
    pub entries: Vec<AuditEntry>,
    pub has_more: bool,
}

/// Events matching `q` (action or e-mail), older than `before` when given.
pub(crate) async fn search_audit(
    ctx: &AppState,
    actor: &AuthUser,
    q: &str,
    before: Option<i64>,
) -> Result<AuditPage> {
    actor.require(Permission::AUDIT_READ)?;
    // One extra row tells us whether anything older exists.
    let fetch = i64::try_from(AUDIT_PAGE + 1).unwrap_or(i64::MAX);
    let mut entries = db::audit::search(&ctx.db, q.trim(), before, fetch).await?;
    let has_more = entries.len() > AUDIT_PAGE;
    entries.truncate(AUDIT_PAGE);
    Ok(AuditPage { entries, has_more })
}
