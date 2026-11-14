//! Administrative operations shared by the operator CLI.
//!
//! The CLI only prompts; connection options, migrations, validation and the
//! SQL for a super-admin account live here so they cannot drift from the app.

use anyhow::{Context as _, bail};
use secrecy::SecretString;
use serde_json::json;
use sqlx::SqlitePool;

use crate::{
    config::DatabaseSettings,
    db::{
        self,
        audit::{AuditAction, audit},
        auth,
    },
    identity::{
        password_policy::{PASSWORD_MAX, PASSWORD_MIN},
        roles::SUPER_ADMIN_ROLE,
    },
    util::time::now_ms,
};

/// Open a SQLite pool with the same URL normalization, directory creation and
/// PRAGMAs as the server.
pub async fn connect(database: &str) -> anyhow::Result<SqlitePool> {
    let url = if database.starts_with("sqlite:") {
        database.to_owned()
    } else {
        format!("sqlite://{database}")
    };
    let settings = DatabaseSettings {
        url: url.into_boxed_str(),
        max_connections: 1,
        busy_timeout_secs: 5,
    };
    db::create_pool(&settings)
        .await
        .context("unable to open database")
}

/// Apply every migration embedded in the library.
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    db::migrate(pool)
        .await
        .context("unable to migrate database")
}

/// Password length bounds, for prompt text.
#[must_use]
pub const fn password_bounds() -> (usize, usize) {
    (PASSWORD_MIN, PASSWORD_MAX)
}

/// Validate a password with the server's policy.
pub fn validate_password(password: &str) -> Result<(), String> {
    crate::identity::password_policy::validate_password(password)
}

/// Validate an e-mail address the way the users table expects.
pub fn validate_email(email: &str) -> Result<(), String> {
    if !(3..=254).contains(&email.len()) {
        return Err("email must be 3-254 characters".to_owned());
    }
    if email.chars().any(char::is_whitespace) {
        return Err("email must not contain whitespace".to_owned());
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err("email must contain '@'".to_owned());
    };
    let dot_ok = domain
        .char_indices()
        .any(|(i, c)| c == '.' && i >= 1 && i < domain.len().saturating_sub(1));
    if local.is_empty() || !dot_ok {
        return Err("email must look like name@example.com".to_owned());
    }
    Ok(())
}

/// Validate a username against the users table's CHECK constraint.
pub fn validate_username(username: &str) -> Result<(), String> {
    if !(3..=32).contains(&username.len()) {
        return Err("username must be 3-32 characters".to_owned());
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err("username may only contain letters, digits, '_', '.', '-'".to_owned());
    }
    Ok(())
}

/// Whether a live user already owns this e-mail address.
pub async fn email_in_use(pool: &SqlitePool, email: &str) -> anyhow::Result<bool> {
    let taken: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM users WHERE email = ? AND deleted_at IS NULL")
            .bind(email)
            .fetch_optional(pool)
            .await?;
    Ok(taken.is_some())
}

/// Whether a live user already owns this username.
pub async fn username_in_use(pool: &SqlitePool, username: &str) -> anyhow::Result<bool> {
    let taken: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM users WHERE username = ? AND deleted_at IS NULL")
            .bind(username)
            .fetch_optional(pool)
            .await?;
    Ok(taken.is_some())
}

/// Number of live super-admin accounts.
pub async fn super_admin_count(pool: &SqlitePool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM user_roles ur
           JOIN roles r ON r.id = ur.role_id
           JOIN users u ON u.id = ur.user_id
          WHERE r.name = ? AND u.deleted_at IS NULL",
    )
    .bind(SUPER_ADMIN_ROLE)
    .fetch_one(pool)
    .await?)
}

#[derive(Debug)]
pub struct CreatedSuperAdmin {
    pub id: i64,
    pub public_id: String,
}

/// Hash a password with the same Argon2 configuration the server uses.
pub async fn hash_password(password: SecretString) -> anyhow::Result<String> {
    Ok(crate::infra::crypto::hash_password(password).await?)
}

/// Create a pre-verified, active super-admin account.
pub async fn create_super_admin(
    pool: &SqlitePool,
    email: &str,
    username: Option<&str>,
    display_name: Option<&str>,
    password_hash: &str,
) -> anyhow::Result<CreatedSuperAdmin> {
    validate_email(email).map_err(anyhow::Error::msg)?;
    if let Some(username) = username {
        validate_username(username).map_err(anyhow::Error::msg)?;
    }
    if email_in_use(pool, email).await? {
        bail!("email is already in use");
    }
    if let Some(username) = username
        && username_in_use(pool, username).await?
    {
        bail!("username is already in use");
    }

    let now = now_ms();
    let mut tx = pool.begin().await?;

    let (user_id, public_id): (i64, String) = sqlx::query_as(
        "INSERT INTO users (email, email_verified_at, username, status, status_reason)
         VALUES (?, ?, ?, 'active', 'created via create_super_admin CLI')
         RETURNING id, public_id",
    )
    .bind(email)
    .bind(now)
    .bind(username)
    .fetch_one(&mut *tx)
    .await?;

    auth::set_display_name(&mut tx, user_id, display_name).await?;
    auth::insert_credentials(&mut tx, user_id, password_hash).await?;

    if !auth::grant_role(&mut tx, user_id, SUPER_ADMIN_ROLE).await? {
        bail!("`{SUPER_ADMIN_ROLE}` role not found; was the seed migration applied?");
    }

    audit(
        &mut *tx,
        None,
        Some(user_id),
        AuditAction::UserCreated,
        Some(json!({ "source": "create_super_admin_cli" })),
    )
    .await;
    audit(
        &mut *tx,
        None,
        Some(user_id),
        AuditAction::RoleGranted,
        Some(json!({ "role": SUPER_ADMIN_ROLE, "source": "create_super_admin_cli" })),
    )
    .await;

    tx.commit().await?;
    Ok(CreatedSuperAdmin {
        id: user_id,
        public_id,
    })
}
