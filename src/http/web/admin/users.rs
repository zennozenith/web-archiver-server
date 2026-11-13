//! User directory: search, details modal, status / role changes, soft delete.

use std::fmt::Write as _;

use askama::Template;
use axum::extract::State;
use serde::Deserialize;

use super::audit::AuditItem;
use crate::{
    AppState,
    auth::{AuthUser, Permission},
    db::user::UserSearch,
    error::AppError,
    http::{
        dto::admin::StatusRequest,
        extract::{
            body::{AppPath, AppQuery},
            web_user::WebUser,
        },
        htmx::{ActionError, Fragments, HxHeaders, PageError, Render, ToastKind},
        web::{
            account::AccountOverview,
            form::ActionForm,
            view::{
                Caps, PAGE_SIZE, Pager, Section, Shell, Stamp, StatusOption, UserItem, pct_encode,
                status_options,
            },
        },
    },
    service::admin,
    util::time::now_ms,
};

/// Closes the details dialog (listened for by `#modal` in `base.templ.html`).
const CLOSE_MODAL: &str = r#"{"modal-close":{"target":"body"}}"#;
const DASH: &str = "\u{2014}";

// ---------------------------------------------------------------- list / page

#[derive(Debug, Deserialize)]
pub(super) struct ListQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    role: String,
    page: Option<String>,
}

fn non_empty(s: &str) -> Option<&str> {
    let s = s.trim();
    (!s.is_empty()).then_some(s)
}

impl ListQuery {
    fn page_number(&self) -> i64 {
        self.page
            .as_deref()
            .and_then(|p| p.parse().ok())
            .unwrap_or(1)
            .max(1)
    }

    fn search(&self) -> UserSearch<'_> {
        UserSearch {
            q: self.q.trim(),
            status: non_empty(&self.status),
            role: non_empty(&self.role),
        }
    }

    /// Query string reproducing the filters, ending in `&` when non-empty.
    fn base_qs(&self) -> String {
        let mut out = String::new();
        for (key, value) in [
            ("q", self.q.trim()),
            ("status", &self.status),
            ("role", &self.role),
        ] {
            if !value.is_empty() {
                let _throw_away = write!(out, "{key}={}&", pct_encode(value));
            }
        }
        out
    }
}

#[derive(Debug)]
pub(crate) struct UsersTable {
    pub(crate) rows: Vec<UserItem>,
    pub(crate) pager: Pager,
}

async fn load_table(
    ctx: &AppState,
    actor: &AuthUser,
    query: &ListQuery,
) -> Result<UsersTable, AppError> {
    let found =
        admin::search_users(ctx, actor, &query.search(), query.page_number(), PAGE_SIZE).await?;
    Ok(UsersTable {
        pager: Pager::new(found.total, found.page, PAGE_SIZE, &query.base_qs()),
        rows: found
            .rows
            .into_iter()
            .map(|u| UserItem::new(u, &actor.public_id))
            .collect(),
    })
}

#[derive(Debug)]
pub(crate) struct RoleFilter {
    pub(crate) name: String,
    pub(crate) selected: bool,
}

#[derive(Debug)]
pub(crate) struct Filters {
    pub(crate) q: String,
    pub(crate) status: String,
    pub(crate) statuses: Vec<StatusOption>,
    pub(crate) roles: Vec<RoleFilter>,
}

#[derive(Debug, Template)]
#[template(path = "users.templ.html")]
pub(crate) struct UsersTpl {
    shell: Shell,
    filters: Filters,
    table: UsersTable,
}

pub(super) async fn page(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
    AppQuery(query): AppQuery<ListQuery>,
) -> Result<Render<UsersTpl>, PageError> {
    let table = load_table(&ctx, &user, &query).await?;
    let roles = admin::role_names(&ctx, &user)
        .await?
        .into_iter()
        .map(|name| RoleFilter {
            selected: name == query.role,
            name,
        })
        .collect();
    Ok(Render(UsersTpl {
        shell: Shell::new::<AccountOverview>(&user, Section::Users),
        filters: Filters {
            statuses: status_options(&query.status),
            q: query.q,
            status: query.status,
            roles,
        },
        table,
    }))
}

#[derive(Debug, Template)]
#[template(path = "partials/users_table.templ.html")]
pub(crate) struct UsersTableTpl {
    table: UsersTable,
}

/// Fragment: table + pager for the current filters (target `#users-results`).
pub(super) async fn table(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
    AppQuery(query): AppQuery<ListQuery>,
) -> Result<Render<UsersTableTpl>, ActionError> {
    Ok(Render(UsersTableTpl {
        table: load_table(&ctx, &user, &query).await?,
    }))
}

// -------------------------------------------------------------- details modal

#[derive(Debug)]
pub(crate) struct Detail {
    pub(crate) login_count: i64,
    pub(crate) last_ip: String,
    pub(crate) status_reason: String,
    pub(crate) failed_attempts: i64,
    pub(crate) locked: Stamp,
    pub(crate) sessions: i64,
    pub(crate) api_keys: i64,
}

#[derive(Debug)]
pub(crate) struct RoleOption {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) assigned: bool,
    pub(crate) locked: bool,
    pub(crate) lock_note: &'static str,
}

#[derive(Debug, Template)]
#[template(path = "partials/user_modal.templ.html")]
pub(crate) struct UserModalTpl {
    u: UserItem,
    detail: Detail,
    roles: Vec<RoleOption>,
    statuses: Vec<StatusOption>,
    caps: Caps,
    activity: Vec<AuditItem>,
}

fn caps_for(viewer: &AuthUser) -> Caps {
    let mut granted = Vec::new();
    if viewer.can(Permission::USERS_WRITE) {
        granted.push("write");
    }
    if viewer.can(Permission::ROLES_WRITE) {
        granted.push("roles");
    }
    if viewer.can(Permission::USERS_DELETE) {
        granted.push("delete");
    }
    if viewer.can(Permission::AUDIT_READ) {
        granted.push("activity");
    }
    Caps::new(granted)
}

async fn build_modal(
    ctx: &AppState,
    viewer: &AuthUser,
    public_id: &str,
) -> Result<UserModalTpl, AppError> {
    let d = admin::user_detail(ctx, viewer, public_id).await?;
    let u = UserItem::new(d.user, &viewer.public_id);

    let locked = match d.extra.locked_until {
        Some(t) if t > now_ms() => Stamp::from_ms(t),
        _ => Stamp::missing("Not locked"),
    };
    let roles = d
        .roles
        .into_iter()
        .map(|r| RoleOption {
            assigned: r.assigned,
            locked: !r.assignable,
            lock_note: if u.is_self {
                "You can't change your own roles"
            } else {
                "Only a super admin can change this"
            },
            description: r.description.unwrap_or_default(),
            name: r.name,
        })
        .collect();

    Ok(UserModalTpl {
        detail: Detail {
            login_count: d.extra.login_count,
            last_ip: d.extra.last_login_ip.unwrap_or_else(|| DASH.to_owned()),
            status_reason: d.extra.status_reason.unwrap_or_default(),
            failed_attempts: d.extra.failed_attempts.unwrap_or(0),
            locked,
            sessions: d.extra.sessions,
            api_keys: d.extra.api_keys,
        },
        roles,
        statuses: status_options(&u.status),
        caps: caps_for(viewer),
        activity: d.activity.into_iter().map(AuditItem::from).collect(),
        u,
    })
}

pub(super) async fn manage(
    State(ctx): State<AppState>,
    WebUser(viewer): WebUser,
    AppPath(public_id): AppPath<String>,
) -> Result<Render<UserModalTpl>, ActionError> {
    Ok(Render(build_modal(&ctx, &viewer, &public_id).await?))
}

// ------------------------------------------------------------------ mutations

/// Out-of-band update of the user's table row.
#[derive(Debug, Template)]
#[template(path = "partials/user_row_oob.templ.html")]
struct UserRowOob {
    u: UserItem,
}

/// Out-of-band removal of the user's table row.
#[derive(Debug, Template)]
#[template(path = "partials/user_row_gone.templ.html")]
struct UserRowGone {
    public_id: String,
}

/// Main swap: the refreshed modal. Out of band: the table row and a toast.
async fn refreshed(
    ctx: &AppState,
    actor: &AuthUser,
    public_id: &str,
    message: &str,
) -> Result<Fragments, AppError> {
    let modal = build_modal(ctx, actor, public_id).await?;
    let row = UserRowOob { u: modal.u.clone() };
    Fragments::new()
        .push(&modal)?
        .push(&row)?
        .toast(ToastKind::Success, message)
}

pub(super) async fn set_status(
    State(ctx): State<AppState>,
    WebUser(actor): WebUser,
    AppPath(public_id): AppPath<String>,
    ActionForm(form): ActionForm<StatusRequest>,
) -> Result<Fragments, ActionError> {
    admin::set_status(
        &ctx,
        &actor,
        &public_id,
        form.status,
        form.reason.as_deref(),
    )
    .await?;
    Ok(refreshed(&ctx, &actor, &public_id, "Status updated").await?)
}

pub(super) async fn grant(
    State(ctx): State<AppState>,
    WebUser(actor): WebUser,
    AppPath((public_id, role)): AppPath<(String, String)>,
) -> Result<Fragments, ActionError> {
    admin::grant_role(&ctx, &actor, &public_id, &role).await?;
    Ok(refreshed(&ctx, &actor, &public_id, &format!("Granted {role}")).await?)
}

pub(super) async fn revoke(
    State(ctx): State<AppState>,
    WebUser(actor): WebUser,
    AppPath((public_id, role)): AppPath<(String, String)>,
) -> Result<Fragments, ActionError> {
    admin::revoke_role(&ctx, &actor, &public_id, &role).await?;
    Ok(refreshed(&ctx, &actor, &public_id, &format!("Revoked {role}")).await?)
}

pub(super) async fn delete(
    State(ctx): State<AppState>,
    WebUser(actor): WebUser,
    AppPath(public_id): AppPath<String>,
) -> Result<Fragments, ActionError> {
    admin::delete_user(&ctx, &actor, &public_id).await?;
    Ok(Fragments::new()
        .push(&UserRowGone { public_id })?
        .hx(HxHeaders::new().trigger(CLOSE_MODAL))
        .toast(ToastKind::Success, "User deleted")?)
}
