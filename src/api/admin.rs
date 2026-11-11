use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, patch, post},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::{
    AppState,
    db::{
        admin::{AuditQuery, ListQuery, StatusReq},
        user::UserRow,
    },
    error::{AppError, ForbiddenReason},
    extract::auth_user::AuthUser,
    time::{ms_to_dt, opt_ms_to_dt},
};

/// Public representation: the integer `id` is never exposed, only `public_id`.
#[derive(Debug, Serialize)]
pub(crate) struct UserDto {
    pub public_id: String,
    pub email: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub username: Option<String>,
    pub phone_e164: Option<String>,
    pub status: String,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub roles: Vec<String>,
}

impl From<UserRow> for UserDto {
    fn from(r: UserRow) -> Self {
        Self {
            public_id: r.public_id,
            email: r.email,
            email_verified_at: opt_ms_to_dt(r.email_verified_at),
            username: r.username,
            phone_e164: r.phone_e164,
            status: r.status,
            last_login_at: opt_ms_to_dt(r.last_login_at),
            created_at: ms_to_dt(r.created_at),
            display_name: r.display_name,
            first_name: r.first_name,
            last_name: r.last_name,
            avatar_url: r.avatar_url,
            locale: r.locale,
            timezone: r.timezone,
            roles: r
                .roles
                .map_or_default(|s| s.split(',').map(str::to_owned).collect()),
        }
    }
}

// ----------------------------------------------------------------- users

async fn list_users(
    State(pool): State<SqlitePool>,
    admin: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    admin.require("users:read")?;

    let rows = crate::db::admin::list_users(&pool, q).await?;

    let users: Vec<UserDto> = rows.into_iter().map(UserDto::from).collect();
    Ok(Json(json!({ "users": users })))
}

async fn set_status(
    State(pool): State<SqlitePool>,
    admin: AuthUser,
    Path(public_id): Path<String>,
    Json(req): Json<StatusReq>,
) -> Result<Json<Value>, AppError> {
    admin.require("users:write")?;
    if !["pending", "active", "suspended", "deactivated"].contains(&req.status.as_str()) {
        return Err(AppError::BadRequest("unknown status".into()));
    }

    crate::db::admin::set_status(&pool, admin.id, public_id, req).await?;

    Ok(Json(json!({ "ok": true })))
}

async fn delete_user(
    State(pool): State<SqlitePool>,
    admin: AuthUser,
    Path(public_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    admin.require("users:delete")?;

    crate::db::admin::delete_user(&pool, admin.id, public_id).await?;

    Ok(Json(json!({ "ok": true })))
}

// ----------------------------------------------------------------- roles

async fn grant_role(
    State(pool): State<SqlitePool>,
    admin: AuthUser,
    Path((public_id, role)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    admin.require("roles:write")?;
    // Only a wildcard holder may hand out the top role.
    if role == "super_admin" && !admin.can("*") {
        return Err(AppError::Forbidden(ForbiddenReason::Permission));
    }

    crate::db::admin::grant_role(&pool, admin.id, public_id, role).await?;

    Ok(Json(json!({ "ok": true })))
}

async fn revoke_role(
    State(pool): State<SqlitePool>,
    admin: AuthUser,
    Path((public_id, role)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    admin.require("roles:write")?;
    if role == "super_admin" && !admin.can("*") {
        return Err(AppError::Forbidden(ForbiddenReason::Permission));
    }

    crate::db::admin::revoke_role(&pool, admin.id, public_id, role).await?;

    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize)]
struct RoleDto {
    name: String,
    description: Option<String>,
    is_system: bool,
    permissions: Vec<String>,
}

async fn list_roles(
    State(pool): State<SqlitePool>,
    admin: AuthUser,
) -> Result<Json<Value>, AppError> {
    admin.require("roles:read")?;

    let rows = crate::db::admin::list_roles(&pool).await?;

    let roles: Vec<RoleDto> = rows
        .into_iter()
        .map(|r| RoleDto {
            name: r.name,
            description: r.description,
            is_system: r.is_system != 0,
            permissions: r
                .permissions
                .map_or_default(|s| s.split(',').map(str::to_owned).collect()),
        })
        .collect();
    Ok(Json(json!({ "roles": roles })))
}

// ----------------------------------------------------------------- audit

#[derive(Serialize)]
struct AuditDto {
    id: i64,
    action: String,
    actor: Option<String>,
    target: Option<String>,
    details: Value,
    created_at: DateTime<Utc>,
}

async fn list_audit(
    State(pool): State<SqlitePool>,
    admin: AuthUser,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Value>, AppError> {
    admin.require("audit:read")?;

    let rows = crate::db::admin::list_audit(&pool, q).await?;

    let entries: Vec<AuditDto> = rows
        .into_iter()
        .map(|r| AuditDto {
            id: r.id,
            action: r.action,
            actor: r.actor_email,
            target: r.target_email,
            details: serde_json::from_str(&r.details).unwrap_or(Value::Null),
            created_at: ms_to_dt(r.created_at),
        })
        .collect();
    Ok(Json(json!({ "entries": entries })))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        // --- admin (RBAC protected)
        .route("/admin/users", get(list_users))
        .route("/admin/users/{public_id}", delete(delete_user))
        .route("/admin/users/{public_id}/status", patch(set_status))
        .route(
            "/admin/users/{public_id}/roles/{role}",
            post(grant_role).delete(revoke_role),
        )
        .route("/admin/roles", get(list_roles))
        .route("/admin/audit", get(list_audit))
}
