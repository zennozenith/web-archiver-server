//! Overview data. What is loaded follows the viewer's permissions:
//! everyone gets "your account"; `users:read` adds user counts and newest
//! sign-ups; `audit:read` adds security counters and recent events.

use crate::{
    db::{
        self, Database,
        stats::{DayCount, SecurityCounts, UserCounts},
    },
    error::{AppError, ForbiddenReason, Result},
    identity::{AuditEntry, AuthUser, Permission, UserFilter, UserView},
    util::time::now_ms,
};

const RECENT: i64 = 6;

pub(crate) struct Security {
    pub counts: SecurityCounts,
    pub days: Vec<DayCount>,
}

pub(crate) struct Overview {
    pub user: UserView,
    pub users: Option<UserCounts>,
    pub recent_users: Vec<UserView>,
    pub security: Option<Security>,
    pub recent_audit: Vec<AuditEntry>,
}

pub(crate) async fn overview(pool: &Database, actor: &AuthUser) -> Result<Overview> {
    if !actor.is_super_admin() {
        return Err(AppError::Forbidden(ForbiddenReason::MissingPermission));
    }

    let read = pool.reader();
    let now = now_ms();
    let user = db::user::find(read, actor.id).await?;

    let (users, recent_users) = if actor.can(Permission::USERS_READ) {
        let counts = db::stats::user_counts(read, now).await?;
        let filter = UserFilter {
            limit: RECENT,
            offset: 0,
            ..UserFilter::new(None, None, None, None)?
        };
        let recent = db::user::search(read, &filter).await?;
        (Some(counts), recent)
    } else {
        (None, Vec::new())
    };

    let (security, recent_audit) = if actor.can(Permission::AUDIT_READ) {
        let counts = db::stats::security_counts(read, now).await?;
        let days = db::stats::login_days(read).await?;
        let recent = db::audit::list(read, RECENT).await?;
        (Some(Security { counts, days }), recent)
    } else {
        (None, Vec::new())
    };

    Ok(Overview {
        user,
        users,
        recent_users,
        security,
        recent_audit,
    })
}
