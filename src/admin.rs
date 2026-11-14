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
    identity::{password_policy, roles::SUPER_ADMIN_ROLE},
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
        ..DatabaseSettings::default()
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
    (password_policy::PASSWORD_MIN, password_policy::PASSWORD_MAX)
}

/// Validate a password with the server's policy.
pub fn validate_password(password: &str) -> Result<(), String> {
    password_policy::validate_password(password)
}

/// Validate an e-mail address with the shared identity rules.
pub fn validate_email(email: &str) -> Result<(), String> {
    crate::identity::validate_email(email)
}

/// Validate a username with the shared identity rules.
pub fn validate_username(username: &str) -> Result<(), String> {
    crate::identity::validate_username(username)
}

/// Whether a live user already owns this e-mail address.
pub async fn email_in_use(pool: &SqlitePool, email: &str) -> anyhow::Result<bool> {
    Ok(db::user::email_in_use(pool, email).await?)
}

/// Whether a live user already owns this username.
pub async fn username_in_use(pool: &SqlitePool, username: &str) -> anyhow::Result<bool> {
    Ok(db::user::username_in_use(pool, username).await?)
}

/// Number of live super-admin accounts.
pub async fn super_admin_count(pool: &SqlitePool) -> anyhow::Result<i64> {
    Ok(db::user::super_admin_count(pool).await?)
}

#[derive(Debug)]
pub struct CreatedSuperAdmin {
    pub id: i64,
    pub public_id: String,
}

/// Everything needed to create a pre-verified, active super-admin account.
#[derive(Debug)]
pub struct NewSuperAdmin<'a> {
    pub email: &'a str,
    pub username: Option<&'a str>,
    pub display_name: Option<&'a str>,
    pub password: SecretString,
}

/// Create a pre-verified, active super-admin account.
pub async fn create_super_admin(
    pool: &SqlitePool,
    account: NewSuperAdmin<'_>,
) -> anyhow::Result<CreatedSuperAdmin> {
    validate_email(account.email).map_err(anyhow::Error::msg)?;
    if let Some(username) = account.username {
        validate_username(username).map_err(anyhow::Error::msg)?;
    }
    if email_in_use(pool, account.email).await? {
        bail!("email is already in use");
    }
    if let Some(username) = account.username
        && username_in_use(pool, username).await?
    {
        bail!("username is already in use");
    }

    // Argon2 is deliberately slow; hash before taking the write transaction.
    let password_hash = crate::infra::crypto::hash_password(account.password).await?;
    let now = now_ms();
    let mut tx = pool.begin().await?;

    let (user_id, public_id) = auth::insert_active_user(
        &mut tx,
        account.email,
        account.username,
        now,
        "created via create_super_admin CLI",
    )
    .await?;

    auth::set_display_name(&mut tx, user_id, account.display_name).await?;
    auth::insert_credentials(&mut tx, user_id, &password_hash).await?;

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
