use axum::{
    Json, Router,
    extract::State,
    routing::{delete, get, patch, post},
};
use serde_json::{Value, json};

use crate::{
    app::AppState,
    auth::{AuthUser, UserFilter},
    error::Result,
    http::{
        dto::{
            OkDto,
            admin::{AuditDto, AuditQuery, ListUsersQuery, RoleDto, StatusRequest},
            user::UserDto,
        },
        extract::body::{AppJson, AppPath, AppQuery},
    },
    service,
};

async fn list_users(
    State(ctx): State<AppState>,
    admin: AuthUser,
    AppQuery(q): AppQuery<ListUsersQuery>,
) -> Result<Json<Value>> {
    let filter = UserFilter::try_from(q)?;
    let users: Vec<UserDto> = service::admin::list_users(&ctx, &admin, &filter)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(json!({ "users": users })))
}

async fn set_status(
    State(ctx): State<AppState>,
    admin: AuthUser,
    AppPath(public_id): AppPath<String>,
    AppJson(req): AppJson<StatusRequest>,
) -> Result<Json<OkDto>> {
    service::admin::set_status(&ctx, &admin, &public_id, req.status, req.reason.as_deref()).await?;
    Ok(Json(OkDto::new()))
}

async fn delete_user(
    State(ctx): State<AppState>,
    admin: AuthUser,
    AppPath(public_id): AppPath<String>,
) -> Result<Json<OkDto>> {
    service::admin::delete_user(&ctx, &admin, &public_id).await?;
    Ok(Json(OkDto::new()))
}

async fn grant_role(
    State(ctx): State<AppState>,
    admin: AuthUser,
    AppPath((public_id, role)): AppPath<(String, String)>,
) -> Result<Json<OkDto>> {
    service::admin::grant_role(&ctx, &admin, &public_id, &role).await?;
    Ok(Json(OkDto::new()))
}

async fn revoke_role(
    State(ctx): State<AppState>,
    admin: AuthUser,
    AppPath((public_id, role)): AppPath<(String, String)>,
) -> Result<Json<OkDto>> {
    service::admin::revoke_role(&ctx, &admin, &public_id, &role).await?;
    Ok(Json(OkDto::new()))
}

async fn list_roles(State(ctx): State<AppState>, admin: AuthUser) -> Result<Json<Value>> {
    let roles: Vec<RoleDto> = service::admin::list_roles(&ctx, &admin)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(json!({ "roles": roles })))
}

async fn list_audit(
    State(ctx): State<AppState>,
    admin: AuthUser,
    AppQuery(q): AppQuery<AuditQuery>,
) -> Result<Json<Value>> {
    let entries: Vec<AuditDto> = service::admin::list_audit(&ctx, &admin, q.limit)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(json!({ "entries": entries })))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
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
