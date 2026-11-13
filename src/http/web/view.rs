//! Display-ready view-models shared by the templates. Everything here is plain
//! data (strings, numbers, flags) so the templates stay free of logic.

use chrono::{DateTime, SecondsFormat, Utc};

use crate::{
    auth::{AuthUser, Permission, UserView},
    util::time::{ms_to_dt, opt_ms_to_dt as _},
};

use std::fmt::Write;

pub(crate) const PAGE_SIZE: i64 = 15;

/// Inner markup of 24x24 stroke icons (rendered with `|safe`; static strings only).
mod icons {
    pub(super) const GRID: &str = r#"<rect x="3.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="3.5" y="13.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="13.5" width="7" height="7" rx="1.5"/>"#;
    pub(super) const USERS: &str = r#"<circle cx="9" cy="8" r="3.25"/><path d="M3 19.5c0-3.3 2.7-5.75 6-5.75s6 2.45 6 5.75"/><circle cx="17.25" cy="9.25" r="2.5"/><path d="M17.5 14.2c2.2.2 3.9 1.9 3.9 4.3"/>"#;
    pub(super) const SHIELD: &str = r#"<path d="M12 3.5l7.5 2.8v5.4c0 4.4-3.1 7.6-7.5 8.8-4.4-1.2-7.5-4.4-7.5-8.8V6.3z"/><path d="M9 12l2.2 2.2L15.2 10"/>"#;
    pub(super) const LEDGER: &str = r#"<rect x="5" y="3.5" width="14" height="17" rx="2"/><path d="M9 8.5h6M9 12h6M9 15.5h3.5"/>"#;
    pub(super) const USER: &str =
        r#"<circle cx="12" cy="8.5" r="3.5"/><path d="M5 20c0-3.6 3.1-6 7-6s7 2.4 7 6"/>"#;
    pub(super) const MONITOR: &str =
        r#"<rect x="3" y="4.5" width="18" height="11.5" rx="2"/><path d="M8.5 20h7M12 16v4"/>"#;
    pub(super) const KEY: &str =
        r#"<circle cx="8" cy="15.5" r="4"/><path d="M11 12.5l9-9M16.5 7l3 3"/>"#;
}

// ------------------------------------------------------------------ app shell

/// Which sidebar entry is highlighted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Dashboard,
    Users,
    Roles,
    Audit,
    Profile,
    Sessions,
    Keys,
}

impl Section {
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Users => "Users",
            Self::Roles => "Roles & permissions",
            Self::Audit => "Audit log",
            Self::Profile => "Profile",
            Self::Sessions => "Sessions",
            Self::Keys => "API keys",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NavItem {
    pub(crate) href: &'static str,
    pub(crate) label: &'static str,
    pub(crate) icon: &'static str,
    pub(crate) active: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct NavGroup {
    pub(crate) label: &'static str,
    pub(crate) items: Vec<NavItem>,
}

/// Everything `base.html` needs: page title, permission-filtered navigation
/// and the signed-in user's chip.
#[derive(Debug, Clone)]
pub(crate) struct Shell {
    pub(crate) title: &'static str,
    pub(crate) groups: Vec<NavGroup>,
    pub(crate) me_name: String,
    pub(crate) me_roles: String,
    pub(crate) me_initials: String,
}

impl Shell {
    pub(crate) fn new(user: &AuthUser, active: Section) -> Self {
        let me_name = user
            .email
            .clone()
            .or_else(|| user.username.clone())
            .unwrap_or_else(|| "Signed in".to_owned());
        Self {
            title: active.title(),
            groups: nav_groups(user, active),
            me_initials: initials(&me_name),
            me_roles: user.roles.join(", "),
            me_name,
        }
    }
}

fn nav_groups(user: &AuthUser, active: Section) -> Vec<NavGroup> {
    let item =
        |section: Section, href: &'static str, label: &'static str, icon: &'static str| NavItem {
            href,
            label,
            icon,
            active: section == active,
        };

    let mut groups = vec![NavGroup {
        label: "Overview",
        items: vec![item(
            Section::Dashboard,
            "/account",
            "Dashboard",
            icons::GRID,
        )],
    }];

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
    groups
}

// -------------------------------------------------------------------- helpers

/// A timestamp prepared for `<time datetime=".." data-local>`: the UTC text is
/// the fallback, `app.js` swaps in the browser's local time.
#[derive(Debug, Clone)]
pub(crate) struct Stamp {
    pub iso: String,
    pub text: String,
}

impl Stamp {
    fn from_dt(dt: DateTime<Utc>) -> Self {
        Self {
            iso: dt.to_rfc3339_opts(SecondsFormat::Secs, true),
            text: dt.format("%-d %b %Y, %H:%M UTC").to_string(),
        }
    }

    pub(crate) fn from_ms(ms: i64) -> Self {
        Self::from_dt(ms_to_dt(ms))
    }

    /// No timestamp: shows `text` (e.g. "Never") and is left alone by `app.js`.
    pub(crate) fn missing(text: &str) -> Self {
        Self {
            iso: String::new(),
            text: text.to_owned(),
        }
    }

    pub(crate) fn from_opt_ms(ms: Option<i64>, missing: &str) -> Self {
        ms.map_or_else(|| Self::missing(missing), Self::from_ms)
    }
}

pub(crate) fn status_class(status: &str) -> &'static str {
    match status {
        "active" => "badge-success",
        "pending" => "badge-warning",
        "suspended" => "badge-error",
        _ => "badge-neutral",
    }
}

pub(crate) fn role_class(role: &str) -> &'static str {
    match role {
        "super_admin" => "badge-primary",
        "admin" => "badge-secondary",
        "moderator" => "badge-accent",
        _ => "badge-neutral",
    }
}

/// Up to two capital letters from a display name / e-mail.
pub(crate) fn initials(name: &str) -> String {
    let s: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if s.is_empty() { "?".to_owned() } else { s }
}

/// Percent-encodes a query-string value (RFC 3986 unreserved characters stay).
pub(crate) fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _throw_away = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Which actions the viewer may take (kept as a list so structs stay small).
/// Templates ask `caps.has("write")`.
#[derive(Debug, Clone)]
pub(crate) struct Caps {
    granted: Vec<&'static str>,
}

impl Caps {
    pub(crate) const fn new(granted: Vec<&'static str>) -> Self {
        Self { granted }
    }

    pub(crate) fn has(&self, key: &str) -> bool {
        self.granted.contains(&key)
    }
}

// ----------------------------------------------------------------- user views

#[derive(Debug, Clone)]
pub(crate) struct RoleChip {
    pub name: String,
    pub class: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct UserItem {
    pub public_id: String,
    pub name: String,
    pub secondary: String,
    pub initials: String,
    pub status: String,
    pub status_class: &'static str,
    pub verified: bool,
    pub roles: Vec<RoleChip>,
    pub last_login: Stamp,
    pub created: Stamp,
    pub is_self: bool,
    pub locale: String,
    pub timezone: String,
}

impl UserItem {
    pub(crate) fn new(u: UserView, viewer: &str) -> Self {
        let roles_list = u.roles();
        let status = u.status.to_string();
        let name = u
            .display_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| u.username.clone())
            .or_else(|| u.email.clone())
            .unwrap_or_else(|| "Unnamed user".to_owned());
        let secondary = u
            .email
            .clone()
            .or_else(|| u.username.as_ref().map(|n| format!("@{n}")))
            .or_else(|| u.phone_e164.clone())
            .unwrap_or_default();
        let roles = roles_list
            .into_iter()
            .map(|r| RoleChip {
                class: role_class(&r),
                name: r,
            })
            .collect();

        Self {
            initials: initials(&name),
            verified: u.email_verified_at.is_some(),
            created: Stamp::from_ms(u.created_at),
            last_login: Stamp::from_opt_ms(u.last_login_at, "Never"),
            is_self: u.public_id == viewer,
            status_class: status_class(&status),
            public_id: u.public_id,
            locale: u.locale.unwrap_or_default(),
            timezone: u.timezone.unwrap_or_default(),
            status,
            name,
            secondary,
            roles,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StatusOption {
    pub(crate) value: &'static str,
    pub(crate) label: &'static str,
    pub(crate) selected: bool,
}

const STATUSES: [(&str, &str); 4] = [
    ("active", "Active"),
    ("pending", "Pending verification"),
    ("suspended", "Suspended"),
    ("deactivated", "Deactivated"),
];

pub(crate) fn status_options(selected: &str) -> Vec<StatusOption> {
    STATUSES
        .iter()
        .map(|&(value, label)| StatusOption {
            value,
            label,
            selected: value == selected,
        })
        .collect()
}

// -------------------------------------------------------------------- pager

#[derive(Debug, Clone)]
pub(crate) struct Pager {
    pub(crate) page: i64,
    pub(crate) total: i64,
    pub(crate) from: i64,
    pub(crate) to: i64,
    pub(crate) has_prev: bool,
    pub(crate) has_next: bool,
    pub(crate) prev_qs: String,
    pub(crate) next_qs: String,
}

impl Pager {
    /// `base_qs` is either empty or ends with `&` (e.g. `q=ann&status=active&`).
    pub(crate) fn new(total: i64, page: i64, per_page: i64, base_qs: &str) -> Self {
        let pages = total
            .saturating_add(per_page.saturating_sub(1))
            .checked_div(per_page)
            .unwrap_or(1)
            .max(1);
        let page = page.clamp(1, pages);
        let from = if total == 0 {
            0
        } else {
            page.saturating_sub(1)
                .saturating_mul(per_page)
                .saturating_add(1)
        };
        let to = page.saturating_mul(per_page).min(total);
        Self {
            page,
            total,
            from,
            to,
            has_prev: page > 1,
            has_next: page < pages,
            prev_qs: format!("{base_qs}page={}", page.saturating_sub(1)),
            next_qs: format!("{base_qs}page={}", page.saturating_add(1)),
        }
    }
}
