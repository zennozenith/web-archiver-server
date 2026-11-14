//! Audit log: searchable, newest first, "load older" pagination.

use askama::Template;
use axum::extract::State;
use serde::Deserialize;

use crate::{
    AppState,
    auth::{AuditEntry, AuthUser},
    http::{
        extract::body::AppQuery,
        htmx::Render,
        web::{
            account::AccountOverview,
            viewmodels::{Section, Shell, Stamp, pct_encode},
        },
    },
    prelude::*,
    service::admin,
};

#[derive(Debug, Clone)]
pub(crate) struct AuditItem {
    pub(crate) id: i64,
    pub(crate) action: String,
    pub(crate) class: &'static str,
    pub(crate) actor: String,
    pub(crate) target: String,
    pub(crate) details: String,
    pub(crate) when: Stamp,
}

fn action_class(action: &str) -> &'static str {
    if action.ends_with("deleted") || action.ends_with("revoked") {
        "badge-error"
    } else if action.contains("status") || action.starts_with("password") {
        "badge-warning"
    } else if action.ends_with("created")
        || action.ends_with("verified")
        || action.ends_with("granted")
    {
        "badge-success"
    } else if action.starts_with("auth.") {
        "badge-info"
    } else {
        "badge-neutral"
    }
}

impl From<AuditEntry> for AuditItem {
    fn from(e: AuditEntry) -> Self {
        Self {
            id: e.id,
            class: action_class(&e.action),
            details: if e.details == "{}" {
                "\u{2014}".to_owned()
            } else {
                e.details
            },
            actor: e.actor_email.unwrap_or_else(|| "system".to_owned()),
            target: e.target_email.unwrap_or_else(|| "\u{2014}".to_owned()),
            when: Stamp::from_ms(e.created_at),
            action: e.action,
        }
    }
}

#[derive(Debug)]
pub(crate) struct AuditRows {
    pub(crate) rows: Vec<AuditItem>,
    pub(crate) has_more: bool,
    pub(crate) more_qs: String,
}

async fn load(
    ctx: &AppState,
    actor: &crate::auth::AuthUser,
    q: &str,
    before: Option<i64>,
) -> Result<AuditRows, crate::error::AppError> {
    let page = admin::search_audit(ctx, actor, q, before).await?;
    let rows: Vec<AuditItem> = page.entries.into_iter().map(AuditItem::from).collect();
    let last_id = rows.last().map_or(0, |r| r.id);
    Ok(AuditRows {
        more_qs: format!("q={}&before={last_id}", pct_encode(q)),
        has_more: page.has_more,
        rows,
    })
}

#[derive(Debug, Deserialize)]
pub(super) struct AuditQuery {
    #[serde(default)]
    q: String,
    before: Option<String>,
}

impl AuditQuery {
    fn before(&self) -> Option<i64> {
        self.before.as_deref().and_then(|b| b.parse().ok())
    }
}

#[derive(Debug, Template)]
#[template(path = "audit.templ.html")]
pub(crate) struct AuditTpl {
    shell: Shell,
    q: String,
    audit: AuditRows,
}

pub(super) async fn page(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<AuditQuery>,
) -> Result<Render<AuditTpl>> {
    let audit = load(&ctx, &user, &query.q, None).await?;
    Ok(Render(AuditTpl {
        shell: Shell::new::<AccountOverview>(&user, Section::Audit),
        q: query.q,
        audit,
    }))
}

#[derive(Debug, Template)]
#[template(path = "partials/audit_rows.templ.html")]
pub(crate) struct AuditRowsTpl {
    audit: AuditRows,
}

/// Fragment: filtered rows (replaces the tbody) or the next older batch.
pub(super) async fn rows(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppQuery(query): AppQuery<AuditQuery>,
) -> Result<Render<AuditRowsTpl>> {
    Ok(Render(AuditRowsTpl {
        audit: load(&ctx, &user, &query.q, query.before()).await?,
    }))
}
