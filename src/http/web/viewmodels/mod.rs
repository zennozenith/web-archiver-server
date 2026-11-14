//! Display-ready view-models shared by the templates. Everything here is plain
//! data (strings, numbers, flags) so the templates stay free of logic.

pub(crate) mod archive;

use chrono::{DateTime, SecondsFormat, Utc};

use crate::{
    auth::{AuthUser, UserView},
    util::time::ms_to_dt,
};

use std::fmt::Write;

pub(crate) const PAGE_SIZE: i64 = 15;

/// Inner markup of 24x24 stroke icons (rendered with `|safe`; static strings only).
pub(crate) mod icons {
    pub(crate) const GRID: &str = r#"<rect x="3.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="3.5" y="13.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="13.5" width="7" height="7" rx="1.5"/>"#;
    pub(crate) const USERS: &str = r#"<circle cx="9" cy="8" r="3.25"/><path d="M3 19.5c0-3.3 2.7-5.75 6-5.75s6 2.45 6 5.75"/><circle cx="17.25" cy="9.25" r="2.5"/><path d="M17.5 14.2c2.2.2 3.9 1.9 3.9 4.3"/>"#;
    pub(crate) const SHIELD: &str = r#"<path d="M12 3.5l7.5 2.8v5.4c0 4.4-3.1 7.6-7.5 8.8-4.4-1.2-7.5-4.4-7.5-8.8V6.3z"/><path d="M9 12l2.2 2.2L15.2 10"/>"#;
    pub(crate) const LEDGER: &str = r#"<rect x="5" y="3.5" width="14" height="17" rx="2"/><path d="M9 8.5h6M9 12h6M9 15.5h3.5"/>"#;
    pub(crate) const USER: &str =
        r#"<circle cx="12" cy="8.5" r="3.5"/><path d="M5 20c0-3.6 3.1-6 7-6s7 2.4 7 6"/>"#;
    pub(crate) const MONITOR: &str =
        r#"<rect x="3" y="4.5" width="18" height="11.5" rx="2"/><path d="M8.5 20h7M12 16v4"/>"#;
    pub(crate) const KEY: &str =
        r#"<circle cx="8" cy="15.5" r="4"/><path d="M11 12.5l9-9M16.5 7l3 3"/>"#;
    pub(crate) const LINK: &str = r#"<path d="M10 13a5 5 0 0 0 7.07 0l2.83-2.83a5 5 0 0 0-7.07-7.07L11 4.93"/><path d="M14 11a5 5 0 0 0-7.07 0L4.1 13.83a5 5 0 0 0 7.07 7.07L13 19.07"/>"#;
    pub(crate) const FOLDERS: &str = r#"<path d="M3.5 7.5A1.5 1.5 0 0 1 5 6h4l1.5 2H19a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z"/>"#;
    pub(crate) const CLOCK: &str = r#"<circle cx="12" cy="12" r="8"/><path d="M12 7.5V12l3 2"/>"#;
    pub(crate) const TAG: &str =
        r#"<path d="M4 4h8l8 8-8 8-8-8z"/><circle cx="8.5" cy="8.5" r="1.5"/>"#;
    pub(crate) const PIN: &str = r#"<path d="M9 4h6l-1 5 3 3v2H7v-2l3-3zM12 14v6"/>"#;
    pub(crate) const INBOX: &str =
        r#"<path d="M4 5h16v10h-4l-2 2h-4l-2-2H4z"/><path d="M4 12h4l1.5 2h5L16 12h4"/>"#;
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
    Archive,
    Links,
    Pinned,
    Unorganized,
    Collections,
    Pending,
    Tags,
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
            Self::Archive => "Web archive",
            Self::Links => "All links",
            Self::Pinned => "Pinned links",
            Self::Unorganized => "Unorganized",
            Self::Collections => "Collections",
            Self::Pending => "Pending queue",
            Self::Tags => "Tags",
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
    pub(crate) account_href: &'static str,
    pub(crate) me_name: String,
    pub(crate) me_roles: String,
    pub(crate) me_initials: String,
    /// Anonymous visitors see the content without the signed-in navigation.
    pub(crate) guest: bool,
}

pub(crate) trait NavProvider {
    fn groups(user: &AuthUser, active: Section) -> Vec<NavGroup>;
}

impl Shell {
    pub(crate) fn new<N: NavProvider>(user: &AuthUser, active: Section) -> Self {
        let me_name = user
            .email
            .clone()
            .or_else(|| user.username.clone())
            .unwrap_or_else(|| "Signed in".to_owned());
        Self {
            title: active.title(),
            account_href: if user.is_super_admin() {
                "/account"
            } else {
                "/account/profile"
            },
            groups: N::groups(user, active),
            me_initials: initials(&me_name),
            me_roles: user.roles.join(", "),
            me_name,
            guest: false,
        }
    }

    /// Shell for pages that do not require sign-in (e.g. a shared collection).
    pub(crate) const fn public(title: &'static str) -> Self {
        Self {
            title,
            account_href: "/account/profile",
            groups: Vec::new(),
            me_name: String::new(),
            me_roles: String::new(),
            me_initials: String::new(),
            guest: true,
        }
    }
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
