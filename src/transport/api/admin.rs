use axum::{
    Router,
    extract::State,
    routing::{delete, get, patch, post},
};
use serde_json::{Value, json};

use crate::{
    app::AppState,
    error::Result,
    identity::{AuthUser, UserFilter},
    service,
    transport::{
        dto::{
            OkDto,
            admin::{AuditDto, AuditQuery, ListUsersQuery, RoleDto, StatusRequest},
            user::UserDto,
        },
        extract::body::{AppJson, AppPath, AppQuery},
    },
};

async fn list_users(
    State(state): State<AppState>,
    admin: AuthUser,
    AppQuery(q): AppQuery<ListUsersQuery>,
) -> Result<AppJson<Value>> {
    let filter = UserFilter::try_from(q)?;
    let users: Vec<UserDto> = service::admin::list_users(&state.db, &admin, &filter)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(AppJson(json!({ "users": users })))
}

async fn set_status(
    State(state): State<AppState>,
    admin: AuthUser,
    AppPath(public_id): AppPath<String>,
    AppJson(req): AppJson<StatusRequest>,
) -> Result<AppJson<OkDto>> {
    service::admin::set_status(
        &state.db,
        &admin,
        &public_id,
        req.status,
        req.reason.as_deref(),
    )
    .await?;
    Ok(AppJson(OkDto::new()))
}

async fn delete_user(
    State(state): State<AppState>,
    admin: AuthUser,
    AppPath(public_id): AppPath<String>,
) -> Result<AppJson<OkDto>> {
    service::admin::delete_user(&state.db, &admin, &public_id).await?;
    Ok(AppJson(OkDto::new()))
}

async fn grant_role(
    State(state): State<AppState>,
    admin: AuthUser,
    AppPath((public_id, role)): AppPath<(String, String)>,
) -> Result<AppJson<OkDto>> {
    service::admin::grant_role(&state.db, &admin, &public_id, &role).await?;
    Ok(AppJson(OkDto::new()))
}

async fn revoke_role(
    State(state): State<AppState>,
    admin: AuthUser,
    AppPath((public_id, role)): AppPath<(String, String)>,
) -> Result<AppJson<OkDto>> {
    service::admin::revoke_role(&state.db, &admin, &public_id, &role).await?;
    Ok(AppJson(OkDto::new()))
}

async fn list_roles(State(state): State<AppState>, admin: AuthUser) -> Result<AppJson<Value>> {
    let roles: Vec<RoleDto> = service::admin::list_roles(&state.db, &admin)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(AppJson(json!({ "roles": roles })))
}

async fn list_audit(
    State(state): State<AppState>,
    admin: AuthUser,
    AppQuery(q): AppQuery<AuditQuery>,
) -> Result<AppJson<Value>> {
    let entries: Vec<AuditDto> = service::admin::list_audit(&state.db, &admin, q.limit)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(AppJson(json!({ "entries": entries })))
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
