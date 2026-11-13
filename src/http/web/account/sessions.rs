use askama::Template;
use axum::extract::State;

use crate::{
    AppState,
    auth::{AuthUser, SessionView},
    error::AppError,
    http::{
        extract::{body::AppPath, web_user::WebUser},
        htmx::{ActionError, Fragments, PageError, Render, ToastKind},
        web::view::{Section, Shell, Stamp},
    },
    service::me,
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

async fn load(ctx: &AppState, user: &AuthUser) -> Result<Vec<SessionItem>, AppError> {
    Ok(me::list_sessions(ctx, user)
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
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
) -> Result<Render<SessionsTpl>, PageError> {
    Ok(Render(SessionsTpl {
        shell: Shell::new(&user, Section::Sessions),
        sessions: load(&ctx, &user).await?,
    }))
}

pub(super) async fn revoke(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
    AppPath(public_id): AppPath<String>,
) -> Result<Fragments, ActionError> {
    me::revoke_session(&ctx, &user, &public_id).await?;
    Ok(Fragments::new()
        .push(&SessionRowsTpl {
            sessions: load(&ctx, &user).await?,
        })?
        .toast(ToastKind::Success, "Session revoked")?)
}
