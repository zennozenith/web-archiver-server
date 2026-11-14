//! Overview page. The service decides what the viewer may see; this module
//! turns counts into bars and segments.

use askama::Template;
use axum::extract::State;

use super::super::view::{RoleChip, Section, Shell, Stamp, UserItem};
use crate::{
    AppState,
    auth::{AuthUser, Permission},
    db::stats::{DayCount, UserCounts},
    http::{
        htmx::Render,
        web::{
            admin::AuditItem,
            view::{NavGroup, NavItem, NavProvider, icons},
        },
    },
    prelude::*,
    service,
};

#[derive(Debug)]
pub(crate) struct Segment {
    pub(crate) label: &'static str,
    pub(crate) count: i64,
    pub(crate) percent: i64,
    pub(crate) class: &'static str,
}

#[derive(Debug)]
pub(crate) struct UserStats {
    pub(crate) total: i64,
    pub(crate) new_week: i64,
    pub(crate) unverified: i64,
    pub(crate) segments: Vec<Segment>,
}

#[derive(Debug)]
pub(crate) struct DayBar {
    pub(crate) label: String,
    pub(crate) ok: i64,
    pub(crate) failed: i64,
    pub(crate) ok_height: i64,
    pub(crate) failed_height: i64,
}

#[derive(Debug)]
pub(crate) struct SecurityStats {
    pub(crate) active_sessions: i64,
    pub(crate) failed_day: i64,
    pub(crate) locked: i64,
    pub(crate) days: Vec<DayBar>,
}

#[derive(Debug)]
pub(crate) struct Mine {
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) status_class: &'static str,
    pub(crate) roles: Vec<RoleChip>,
    pub(crate) last_login: Stamp,
    pub(crate) sessions: usize,
    pub(crate) api_keys: usize,
}

#[derive(Debug, Template)]
#[template(path = "account_overview.templ.html")]
pub(crate) struct OverviewTpl {
    shell: Shell,
    mine_name: String,
    users: Option<UserStats>,
    recent_users: Vec<UserItem>,
    security: Option<SecurityStats>,
    recent_audit: Vec<AuditItem>,
}

pub(crate) struct AccountOverview;

impl NavProvider for AccountOverview {
    fn groups(user: &crate::auth::AuthUser, active: Section) -> Vec<NavGroup> {
        let item = |section: Section,
                    href: &'static str,
                    label: &'static str,
                    icon: &'static str| NavItem {
            href,
            label,
            icon,
            active: section == active,
        };

        let mut groups = vec![];

        if user.is_super() {
            groups.push(NavGroup {
                label: "Overview",
                items: vec![item(
                    Section::Dashboard,
                    "/account",
                    "Dashboard",
                    icons::GRID,
                )],
            });
        }

        groups.push(NavGroup {
            label: "My account",
            items: vec![
                item(Section::Profile, "/account/profile", "Profile", icons::USER),
                item(
                    Section::Sessions,
                    "/account/sessions",
                    "Sessions",
                    icons::MONITOR,
                ),
                item(Section::Keys, "/account/keys", "API keys", icons::KEY),
            ],
        });

        let mut admin = Vec::new();
        if user.can(Permission::USERS_READ) {
            admin.push(item(Section::Users, "/admin/users", "Users", icons::USERS));
        }
        if user.can(Permission::ROLES_READ) {
            admin.push(item(Section::Roles, "/admin/roles", "Roles", icons::SHIELD));
        }
        if user.can(Permission::AUDIT_READ) {
            admin.push(item(
                Section::Audit,
                "/admin/audit",
                "Audit log",
                icons::LEDGER,
            ));
        }
        if !admin.is_empty() {
            groups.push(NavGroup {
                label: "Administration",
                items: admin,
            });
        }

        groups
    }
}

fn percent(count: i64, total: i64) -> i64 {
    count.saturating_mul(100).checked_div(total).unwrap_or(0)
}

impl From<UserCounts> for UserStats {
    fn from(c: UserCounts) -> Self {
        let segment = |label, count, class| Segment {
            label,
            count,
            percent: percent(count, c.total),
            class,
        };
        Self {
            total: c.total,
            new_week: c.new_week,
            unverified: c.unverified,
            segments: vec![
                segment("Active", c.active, "seg-active"),
                segment("Pending", c.pending, "seg-pending"),
                segment("Suspended", c.suspended, "seg-suspended"),
                segment("Deactivated", c.deactivated, "seg-deactivated"),
            ],
        }
    }
}

fn day_bars(days: Vec<DayCount>) -> Vec<DayBar> {
    let peak = days
        .iter()
        .map(|d| d.ok.saturating_add(d.failed))
        .max()
        .unwrap_or(0)
        .max(1);
    days.into_iter()
        .map(|d| DayBar {
            label: d.date.format("%a").to_string(),
            ok: d.ok,
            failed: d.failed,
            ok_height: percent(d.ok, peak),
            failed_height: percent(d.failed, peak),
        })
        .collect()
}

pub(super) async fn page(
    State(ctx): State<AppState>,
    user: AuthUser,
) -> Result<Render<OverviewTpl>> {
    let overview = service::dashboard::overview(&ctx, &user).await?;

    let me = UserItem::new(overview.user, &user.public_id);

    Ok(Render(OverviewTpl {
        shell: Shell::new::<AccountOverview>(&user, Section::Dashboard),
        mine_name: me.name,
        users: overview.users.map(UserStats::from),
        recent_users: overview
            .recent_users
            .into_iter()
            .map(|u| UserItem::new(u, &user.public_id))
            .collect(),
        security: overview.security.map(|s| SecurityStats {
            active_sessions: s.counts.active_sessions,
            failed_day: s.counts.failed_day,
            locked: s.counts.locked,
            days: day_bars(s.days),
        }),
        recent_audit: overview
            .recent_audit
            .into_iter()
            .map(AuditItem::from)
            .collect(),
    }))
}
