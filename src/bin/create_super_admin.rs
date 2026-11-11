//! Interactive CLI: create a `super_admin` account.
//!
//!   cargo run --features admin-cli --bin create_super_admin -- --database sqlite://data/db/app.db --migrate
//!
//! Options:
//!   --database <url|path>   SQLite URL/path (default: $DATABASE_URL, else sqlite://app.db)
//!   --migrate               Run ./migrations first (useful on a fresh database)

use std::error::Error;
use std::io::{self, Write};
use std::str::FromStr;
use std::time::Duration;

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use secrecy::{ExposeSecret, SecretString};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

type BoxResult<T> = Result<T, Box<dyn Error>>;

const MIN_PASSWORD_LEN: usize = 12;

// Argon2id parameters (OWASP-recommended minimum: m=19 MiB, t=2, p=1).
const ARGON_M_KIB: u32 = 19_456;
const ARGON_T: u32 = 2;
const ARGON_P: u32 = 1;
const ARGON_OUT_LEN: usize = 32;
const SALT_LEN: usize = 16;

// ----------------------------------------------------------------------------
// Entry point
// ----------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("\nError: {e}");
        std::process::exit(1);
    }
}

async fn run() -> BoxResult<()> {
    let args = parse_args()?;
    let pool = connect(&args.database).await?;

    if args.migrate {
        println!("Running migrations...");
        sqlx::migrate!("./migrations").run(&pool).await?;
    }

    ensure_schema(&pool).await?;
    warn_if_super_admin_exists(&pool).await?;

    println!("\n=== Create super admin ===\n");

    let email = prompt_email(&pool).await?;
    let username = prompt_username(&pool).await?;
    let display_name = prompt_optional("Display name (optional): ")?;
    let password = prompt_password()?;

    println!("\nAbout to create:");
    println!("  email:        {email}");
    println!("  username:     {}", username.as_deref().unwrap_or("-"));
    println!("  display name: {}", display_name.as_deref().unwrap_or("-"));
    println!("  role:         super_admin");
    if !confirm("Proceed? [y/N]: ")? {
        println!("Aborted.");
        return Ok(());
    }

    // Argon2 is CPU/memory heavy; keep it off the async executor.
    let hash = tokio::task::spawn_blocking(move || hash_password(&password)).await??;

    let (id, public_id) = create_super_admin(
        &pool,
        &email,
        username.as_deref(),
        display_name.as_deref(),
        &hash,
    )
    .await?;

    println!("\nSuper admin created.");
    println!("  id:        {id}");
    println!("  public_id: {public_id}");
    Ok(())
}

// ----------------------------------------------------------------------------
// CLI args
// ----------------------------------------------------------------------------

struct Args {
    database: String,
    migrate: bool,
}

fn parse_args() -> BoxResult<Args> {
    let mut database = std::env::var("DATABASE_URL").ok();
    let mut migrate = false;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--database" | "-d" => {
                database = Some(it.next().ok_or("--database requires a value")?);
            }
            "--migrate" => migrate = true,
            "--help" | "-h" => {
                println!(
                    "Usage: create_super_admin [--database <url|path>] [--migrate]\n\n\
                     Defaults to $DATABASE_URL, else sqlite://app.db"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        database: database.unwrap_or_else(|| "sqlite://app.db".to_string()),
        migrate,
    })
}

// ----------------------------------------------------------------------------
// Database
// ----------------------------------------------------------------------------

async fn connect(database: &str) -> BoxResult<SqlitePool> {
    let url = if database.starts_with("sqlite:") {
        database.to_string()
    } else {
        format!("sqlite://{database}")
    };

    // Mirror the per-connection PRAGMAs the app sets in src/db.rs.
    // Foreign keys MUST be on or ON DELETE / REFERENCES won't be enforced.
    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));

    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?)
}

async fn ensure_schema(pool: &SqlitePool) -> BoxResult<()> {
    let has_users: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'users'")
            .fetch_optional(pool)
            .await?;

    if has_users.is_none() {
        return Err(
            "`users` table not found. Run your migrations first (or pass --migrate).".into(),
        );
    }

    let role: Option<i64> = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'super_admin'")
        .fetch_optional(pool)
        .await?;
    if role.is_none() {
        return Err("`super_admin` role not found. Was the seed migration applied?".into());
    }
    Ok(())
}

async fn warn_if_super_admin_exists(pool: &SqlitePool) -> BoxResult<()> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM user_roles ur
           JOIN roles r ON r.id = ur.role_id
           JOIN users u ON u.id = ur.user_id
          WHERE r.name = 'super_admin' AND u.deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?;

    if count > 0 {
        println!("Note: {count} super admin account(s) already exist.");
        if !confirm("Create another one? [y/N]: ")? {
            println!("Aborted.");
            std::process::exit(0);
        }
    }
    Ok(())
}

async fn create_super_admin(
    pool: &SqlitePool,
    email: &str,
    username: Option<&str>,
    display_name: Option<&str>,
    password_hash: &str,
) -> BoxResult<(i64, String)> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut tx = pool.begin().await?;

    // Pre-verified, active account (an operator is creating this by hand).
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

    // trg_users_create_profile already inserted an empty profile row.
    if let Some(name) = display_name {
        sqlx::query("UPDATE user_profiles SET display_name = ? WHERE user_id = ?")
            .bind(name)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query(
        "INSERT INTO user_credentials (user_id, password_hash, password_algo)
         VALUES (?, ?, 'argon2id')",
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(&mut *tx)
    .await?;

    let role_id: i64 = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'super_admin'")
        .fetch_one(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO user_roles (user_id, role_id, granted_by) VALUES (?, ?, NULL)")
        .bind(user_id)
        .bind(role_id)
        .execute(&mut *tx)
        .await?;

    // Audit trail (append-only table).
    for (action, entity_type, entity_id) in [
        ("user.created", "user", user_id.to_string()),
        ("role.granted", "role", "super_admin".to_string()),
    ] {
        sqlx::query(
            "INSERT INTO audit_log (actor_user_id, target_user_id, action, entity_type, entity_id, details)
             VALUES (NULL, ?, ?, ?, ?, json_object('source', 'create_super_admin_cli'))",
        )
        .bind(user_id)
        .bind(action)
        .bind(entity_type)
        .bind(entity_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok((user_id, public_id))
}

// ----------------------------------------------------------------------------
// Password hashing (Argon2id -> PHC string, as the schema expects)
// ----------------------------------------------------------------------------

// Returns `String` as the error (not `Box<dyn Error>`) so the result is `Send`
// and can cross the `spawn_blocking` boundary.
fn hash_password(password: &SecretString) -> Result<String, String> {
    let params = Params::new(ARGON_M_KIB, ARGON_T, ARGON_P, Some(ARGON_OUT_LEN))
        .map_err(|e| format!("invalid argon2 params: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt).map_err(|e| format!("failed to get random salt: {e}"))?;

    let mut out = [0u8; ARGON_OUT_LEN];
    argon2
        .hash_password_into(password.expose_secret().as_bytes(), &salt, &mut out)
        .map_err(|e| format!("argon2 hashing failed: {e}"))?;

    // $argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>   (unpadded standard base64)
    Ok(format!(
        "$argon2id$v=19$m={ARGON_M_KIB},t={ARGON_T},p={ARGON_P}${}${}",
        STANDARD_NO_PAD.encode(salt),
        STANDARD_NO_PAD.encode(out),
    ))
}

// ----------------------------------------------------------------------------
// Prompts & validation (mirrors the CHECK constraints in the users table)
// ----------------------------------------------------------------------------

fn read_line(prompt: &str) -> BoxResult<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut buf = String::new();
    if io::stdin().read_line(&mut buf)? == 0 {
        return Err("unexpected end of input".into());
    }
    Ok(buf.trim().to_string())
}

fn prompt_optional(prompt: &str) -> BoxResult<Option<String>> {
    let s = read_line(prompt)?;
    Ok(if s.is_empty() { None } else { Some(s) })
}

fn confirm(prompt: &str) -> BoxResult<bool> {
    let s = read_line(prompt)?.to_ascii_lowercase();
    Ok(matches!(s.as_str(), "y" | "yes"))
}

async fn prompt_email(pool: &SqlitePool) -> BoxResult<String> {
    loop {
        let email = read_line("Email: ")?;
        if let Err(msg) = validate_email(&email) {
            eprintln!("  ✗ {msg}");
            continue;
        }
        let taken: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM users WHERE email = ? AND deleted_at IS NULL")
                .bind(&email)
                .fetch_optional(pool)
                .await?;
        if taken.is_some() {
            eprintln!("  ✗ That email is already in use.");
            continue;
        }
        return Ok(email);
    }
}

async fn prompt_username(pool: &SqlitePool) -> BoxResult<Option<String>> {
    loop {
        let Some(username) = prompt_optional("Username (optional): ")? else {
            return Ok(None);
        };
        if let Err(msg) = validate_username(&username) {
            eprintln!("  ✗ {msg}");
            continue;
        }
        let taken: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM users WHERE username = ? AND deleted_at IS NULL")
                .bind(&username)
                .fetch_optional(pool)
                .await?;
        if taken.is_some() {
            eprintln!("  ✗ That username is already in use.");
            continue;
        }
        return Ok(Some(username));
    }
}

fn prompt_password() -> BoxResult<SecretString> {
    loop {
        let pw = SecretString::from(rpassword::prompt_password("Password: ")?);
        if let Err(msg) = validate_password(pw.expose_secret()) {
            eprintln!("  ✗ {msg}");
            continue;
        }
        let again = SecretString::from(rpassword::prompt_password("Confirm password: ")?);
        if pw.expose_secret() != again.expose_secret() {
            eprintln!("  ✗ Passwords do not match.");
            continue;
        }
        return Ok(pw);
    }
}

fn validate_email(email: &str) -> Result<(), &'static str> {
    if !(3..=254).contains(&email.len()) {
        return Err("Email must be 3-254 characters.");
    }
    if email.chars().any(char::is_whitespace) {
        return Err("Email must not contain whitespace.");
    }
    // Equivalent of the schema's  LIKE '_%@_%._%'
    let Some((local, domain)) = email.split_once('@') else {
        return Err("Email must contain '@'.");
    };
    let dot_ok = domain
        .char_indices()
        .any(|(i, c)| c == '.' && i >= 1 && i < domain.len() - 1);
    if local.is_empty() || !dot_ok {
        return Err("Email must look like name@example.com.");
    }
    Ok(())
}

fn validate_username(u: &str) -> Result<(), &'static str> {
    if !(3..=32).contains(&u.len()) {
        return Err("Username must be 3-32 characters.");
    }
    if !u
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err("Username may only contain letters, digits, '_', '.', '-'.");
    }
    Ok(())
}

fn validate_password(p: &str) -> Result<(), String> {
    if p.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!(
            "Password must be at least {MIN_PASSWORD_LEN} characters."
        ));
    }
    let has_lower = p.chars().any(|c| c.is_lowercase());
    let has_upper = p.chars().any(|c| c.is_uppercase());
    let has_digit = p.chars().any(|c| c.is_ascii_digit());
    let has_other = p.chars().any(|c| !c.is_alphanumeric());
    let classes = [has_lower, has_upper, has_digit, has_other]
        .iter()
        .filter(|b| **b)
        .count();
    if classes < 3 {
        return Err("Use at least 3 of: lowercase, uppercase, digits, symbols.".into());
    }
    Ok(())
}
