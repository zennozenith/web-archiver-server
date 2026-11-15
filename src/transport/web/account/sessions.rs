use askama::Template;
use axum::extract::State;

use crate::{
    AppState,
    error::AppError,
    identity::{AuthUser, SessionView},
    prelude::*,
    service::me,
    transport::{
        extract::body::AppPath,
        htmx::{Fragments, Render, ToastKind},
        web::{
            account::overview::AccountOverview,
            viewmodels::{Section, Shell, Stamp},
        },
    },
};

#[derive(Debug)]
pub(crate) struct SessionItem {
    pub(crate) public_id: String,
    pub(crate) device: String,
    pub(crate) user_agent: String,
    pub(crate) ip: String,
    pub(crate) created: Stamp,
    pub(crate) last_used: Stamp,
    pub(crate) expires: Stamp,
    pub(crate) current: bool,
}

/// "Chrome on Windows" from a raw user-agent string (best effort).
fn describe_agent(ua: &str) -> String {
    let browser = [
        ("Edg", "Edge"),
        ("Firefox", "Firefox"),
        ("Chrome", "Chrome"),
        ("Safari", "Safari"),
        ("curl", "curl"),
    ]
    .iter()
    .find(|(needle, _)| ua.contains(needle))
    .map_or("Unknown browser", |&(_, name)| name);

    let os = [
        ("Android", "Android"),
        ("iPhone", "iOS"),
        ("iPad", "iOS"),
        ("Windows", "Windows"),
        ("Mac OS", "macOS"),
        ("Linux", "Linux"),
    ]
    .iter()
    .find(|(needle, _)| ua.contains(needle))
    .map(|&(_, name)| name);

    os.map_or_else(|| browser.to_owned(), |os| format!("{browser} on {os}"))
}

impl From<SessionView> for SessionItem {
    fn from(s: SessionView) -> Self {
        let ua = s.user_agent.unwrap_or_default();
        Self {
            device: describe_agent(&ua),
            ip: s.ip_address.unwrap_or_else(|| "\u{2014}".to_owned()),
            created: Stamp::from_ms(s.created_at),
            last_used: Stamp::from_opt_ms(s.last_used_at, "Never"),
            expires: Stamp::from_ms(s.expires_at),
            current: s.current,
            public_id: s.public_id,
            user_agent: ua,
        }
    }
}

async fn load(state: &AppState, user: &AuthUser) -> Result<Vec<SessionItem>, AppError> {
    Ok(me::list_sessions(&state.db, user)
        .await?
        .into_iter()
        .map(SessionItem::from)
        .collect())
}

#[derive(Debug, Template)]
#[template(path = "sessions.templ.html")]
pub(crate) struct SessionsTpl {
    shell: Shell,
    sessions: Vec<SessionItem>,
}

#[derive(Debug, Template)]
#[template(path = "partials/session_rows.templ.html")]
struct SessionRowsTpl {
    sessions: Vec<SessionItem>,
}

pub(super) async fn page(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Render<SessionsTpl>> {
    Ok(Render(SessionsTpl {
        shell: Shell::new::<AccountOverview>(&user, Section::Sessions),
        sessions: load(&state, &user).await?,
    }))
}

pub(super) async fn revoke(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(public_id): AppPath<String>,
) -> Result<Fragments> {
    me::revoke_session(&state.db, &user, &public_id).await?;
    Fragments::new()
        .push(&SessionRowsTpl {
            sessions: load(&state, &user).await?,
        })?
        .toast(ToastKind::Success, "Session revoked")
}
