//! Roles & permissions overview (read-only).

use askama::Template;
use axum::extract::State;

use crate::{
    AppState,
    auth::{AuthUser, Permission},
    db::role::RoleSummary,
    http::{
        htmx::Render,
        web::{
            account::AccountOverview,
            viewmodels::{Section, Shell, pct_encode, role_class},
        },
    },
    prelude::*,
    service::admin,
};

#[derive(Debug)]
pub(crate) struct RoleCard {
    pub(crate) name: String,
    pub(crate) class: &'static str,
    pub(crate) description: String,
    pub(crate) system: bool,
    pub(crate) members: i64,
    pub(crate) users_href: String,
    pub(crate) permissions: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct Cell {
    pub(crate) symbol: &'static str,
    pub(crate) class: &'static str,
    pub(crate) label: &'static str,
}

#[derive(Debug)]
pub(crate) struct MatrixRow {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) cells: Vec<Cell>,
}

impl From<RoleSummary> for RoleCard {
    fn from(r: RoleSummary) -> Self {
        Self {
            class: role_class(&r.name),
            users_href: format!("/admin/users?role={}", pct_encode(&r.name)),
            permissions: r.permissions(),
            description: r.description.unwrap_or_default(),
            system: r.is_system,
            members: r.members,
            name: r.name,
        }
    }
}

fn cell(role_perms: &[String], permission: &str) -> Cell {
    if role_perms.iter().any(|p| p == permission) {
        Cell {
            symbol: "\u{25CF}",
            class: "cell-yes",
            label: "Granted",
        }
    } else if role_perms
        .iter()
        .any(|p| p == Permission::WILDCARD.as_str())
    {
        Cell {
            symbol: "\u{25CF}",
            class: "cell-implied",
            label: "Granted through the wildcard permission",
        }
    } else {
        Cell {
            symbol: "\u{2013}",
            class: "cell-no",
            label: "Not granted",
        }
    }
}

#[derive(Debug, Template)]
#[template(path = "roles.templ.html")]
pub(crate) struct RolesTpl {
    shell: Shell,
    cards: Vec<RoleCard>,
    matrix: Vec<MatrixRow>,
    can_users: bool,
}

pub(super) async fn page(State(ctx): State<AppState>, user: AuthUser) -> Result<Render<RolesTpl>> {
    let catalog = admin::role_catalog(&ctx, &user).await?;
    let cards: Vec<RoleCard> = catalog.roles.into_iter().map(RoleCard::from).collect();
    let matrix = catalog
        .permissions
        .into_iter()
        .map(|p| MatrixRow {
            cells: cards
                .iter()
                .map(|c| cell(&c.permissions, &p.name))
                .collect(),
            description: p.description.unwrap_or_default(),
            name: p.name,
        })
        .collect();

    Ok(Render(RolesTpl {
        shell: Shell::new::<AccountOverview>(&user, Section::Roles),
        can_users: user.can(Permission::USERS_READ),
        cards,
        matrix,
    }))
}
