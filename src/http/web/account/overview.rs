//! Overview page. The service decides what the viewer may see; this module
//! turns counts into bars and segments.

use askama::Template;
use axum::extract::State;

use super::super::view::{RoleChip, Section, Shell, Stamp, UserItem};
use crate::{
    AppState,
    db::stats::{DayCount, UserCounts},
    http::{
        extract::web_user::WebUser,
        htmx::{PageError, Render},
        web::admin::AuditItem,
    },
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
#[template(path = "dashboard.templ.html")]
pub(crate) struct DashboardTpl {
    shell: Shell,
    mine: Mine,
    users: Option<UserStats>,
    recent_users: Vec<UserItem>,
    security: Option<SecurityStats>,
    recent_audit: Vec<AuditItem>,
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
    WebUser(user): WebUser,
) -> Result<Render<DashboardTpl>, PageError> {
    let o = service::dashboard::overview(&ctx, &user).await?;

    let me = UserItem::new(o.mine.user, &user.public_id);
    let mine = Mine {
        name: me.name,
        status: me.status,
        status_class: me.status_class,
        roles: me.roles,
        last_login: me.last_login,
        sessions: o.mine.sessions,
        api_keys: o.mine.api_keys,
    };

    Ok(Render(DashboardTpl {
        shell: Shell::new(&user, Section::Dashboard),
        mine,
        users: o.users.map(UserStats::from),
        recent_users: o
            .recent_users
            .into_iter()
            .map(|u| UserItem::new(u, &user.public_id))
            .collect(),
        security: o.security.map(|s| SecurityStats {
            active_sessions: s.counts.active_sessions,
            failed_day: s.counts.failed_day,
            locked: s.counts.locked,
            days: day_bars(s.days),
        }),
        recent_audit: o.recent_audit.into_iter().map(AuditItem::from).collect(),
    }))
}
