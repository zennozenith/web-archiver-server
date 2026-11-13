//! Overview data. What is loaded follows the viewer's permissions:
//! everyone gets "your account"; `users:read` adds user counts and newest
//! sign-ups; `audit:read` adds security counters and recent events.

use crate::{
    app::AppState,
    auth::{AuditEntry, AuthUser, Permission, UserView},
    db::{
        self,
        stats::{DayCount, SecurityCounts, UserCounts},
        user::UserSearch,
    },
    error::Result,
    util::time::now_ms,
};

const RECENT: i64 = 6;

pub(crate) struct Mine {
    pub user: UserView,
    pub sessions: usize,
    pub api_keys: usize,
}

pub(crate) struct Security {
    pub counts: SecurityCounts,
    pub days: Vec<DayCount>,
}

pub(crate) struct Overview {
    pub mine: Mine,
    pub users: Option<UserCounts>,
    pub recent_users: Vec<UserView>,
    pub security: Option<Security>,
    pub recent_audit: Vec<AuditEntry>,
}

pub(crate) async fn overview(ctx: &AppState, actor: &AuthUser) -> Result<Overview> {
    let now = now_ms();
    let (user, sessions, keys) = tokio::try_join!(
        db::user::find(&ctx.db, actor.id),
        db::session::list_for_user(&ctx.db, actor.id),
        db::api_key::list_for_user(&ctx.db, actor.id),
    )?;
    let mine = Mine {
        user,
        sessions: sessions.len(),
        api_keys: keys.len(),
    };

    let (users, recent_users) = if actor.can(Permission::USERS_READ) {
        let counts = db::stats::user_counts(&ctx.db, now).await?;
        let recent = db::user::search(&ctx.db, &UserSearch::default(), RECENT, 0).await?;
        (Some(counts), recent)
    } else {
        (None, Vec::new())
    };

    let (security, recent_audit) = if actor.can(Permission::AUDIT_READ) {
        let counts = db::stats::security_counts(&ctx.db, now).await?;
        let days = db::stats::login_days(&ctx.db).await?;
        let recent = db::audit::list(&ctx.db, RECENT).await?;
        (Some(Security { counts, days }), recent)
    } else {
        (None, Vec::new())
    };

    Ok(Overview {
        mine,
        users,
        recent_users,
        security,
        recent_audit,
    })
}
