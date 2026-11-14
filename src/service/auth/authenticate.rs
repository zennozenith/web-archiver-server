use std::{collections::HashSet, time::Duration};

use secrecy::ExposeSecret;
use sqlx::SqlitePool;

use crate::{
    db,
    error::{AppError, Result},
    identity::{AuthMethod, AuthUser, Credential, Permission},
    infra::sha256_hex,
    util::time::{duration_ms, now_ms},
};

/// Only write `last_used_at` when it is older than this, so ordinary GETs
/// don't take SQLite's single write lock.
const TOUCH_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub(crate) async fn authenticate(pool: &SqlitePool, cred: Credential) -> Result<AuthUser> {
    let now = now_ms();
    match cred {
        Credential::Session(token) => {
            let hit = db::auth::find_session(pool, &sha256_hex(token.expose_secret()))
                .await?
                .ok_or(AppError::Unauthorized)?;

            if now.saturating_sub(hit.last_used_at) > duration_ms(TOUCH_INTERVAL) {
                db::auth::touch_session(pool, &hit.session_public_id, now).await?;
            }

            let (roles, perms) = db::auth::roles_and_permissions(pool, hit.user.id).await?;
            let origin = AuthMethod::Session {
                public_id: hit.session_public_id,
            };
            Ok(AuthUser::new(hit.user, roles, perms, origin))
        }

        Credential::ApiKey(key) => {
            let hit = db::auth::find_api_key(pool, &sha256_hex(key.expose_secret()), now)
                .await?
                .ok_or(AppError::Unauthorized)?;

            // Same throttle applies; add `last_used_at` to the SELECT if you want it.
            db::auth::touch_api_key(pool, hit.key_id, now).await?;

            let scopes: HashSet<String> = serde_json::from_str(&hit.scopes).unwrap_or_default();
            let (roles, owner_perms) = db::auth::roles_and_permissions(pool, hit.user.id).await?;

            // A key never exceeds its owner, and is further limited to its own scopes.
            let perms = if owner_perms.contains(Permission::WILDCARD.as_str()) {
                scopes
            } else {
                owner_perms.intersection(&scopes).cloned().collect()
            };
            Ok(AuthUser::new(hit.user, roles, perms, AuthMethod::ApiKey))
        }
    }
}
