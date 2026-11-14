//! Interactive CLI: create a `super_admin` account.
//!
//!   cargo run --features admin-cli --bin `create_super_admin` -- --database <sqlite://data/db/app.db> --migrate
//!
//! Options:
//!   --database <url|path>   SQLite URL/path (default: `$DATABASE_URL`, else <sqlite://app.db>)
//!   --migrate               Run ./migrations first (useful on a fresh database)
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::disallowed_macros,
    clippy::exit,
    reason = "interactive operator CLI: console output and process exit are the interface"
)]

use std::io::{self, Write};

use anyhow::{Result, bail};
use secrecy::{ExposeSecret, SecretString};
use sqlx::SqlitePool;
use web_archiver::admin;

// ----------------------------------------------------------------------------
// Entry point
// ----------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("\nError: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args = parse_args()?;
    let pool = admin::connect(&args.database).await?;

    if args.migrate {
        println!("Running migrations...");
        admin::migrate(&pool).await?;
    }

    warn_if_super_admin_exists(&pool).await?;

    println!("\n=== Create super admin ===\n");

    let email = if let Some(v) = args.email {
        v
    } else {
        prompt_email(&pool).await?
    };
    let username = if args.username.is_some() {
        args.username
    } else {
        prompt_username(&pool).await?
    };
    let display_name = if args.display_name.is_some() {
        args.display_name
    } else {
        prompt_optional("Display name (optional): ")?
    };
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

    let created = admin::create_super_admin(
        &pool,
        admin::NewSuperAdmin {
            email: &email,
            username: username.as_deref(),
            display_name: display_name.as_deref(),
            password,
        },
    )
    .await?;

    println!("\nSuper admin created.");
    println!("  id:        {}", created.id);
    println!("  public_id: {}", created.public_id);
    Ok(())
}

// ----------------------------------------------------------------------------
// CLI args
// ----------------------------------------------------------------------------

struct Args {
    database: String,
    migrate: bool,
    email: Option<String>,
    username: Option<String>,
    display_name: Option<String>,
}

fn parse_args() -> Result<Args> {
    let mut database = std::env::var("DATABASE_URL").ok();
    let mut migrate = false;
    let mut email = None;
    let mut username = None;
    let mut display_name = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--database" | "-d" => {
                database = Some(
                    it.next()
                        .ok_or_else(|| anyhow::anyhow!("--database requires a value"))?,
                );
            }
            "--migrate" => migrate = true,
            "--email" => email = it.next(),
            "--username" => username = it.next(),
            "--display-name" => display_name = it.next(),
            "--help" | "-h" => {
                println!(
                    "Usage: create_super_admin [--database <url|path>] [--migrate]\n\n\
                     Defaults to $DATABASE_URL, else sqlite://app.db"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    Ok(Args {
        database: database.unwrap_or_else(|| "sqlite://app.db".to_owned()),
        migrate,
        email,
        username,
        display_name,
    })
}

// ----------------------------------------------------------------------------
// Prompts
// ----------------------------------------------------------------------------

fn read_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut buf = String::new();
    if io::stdin().read_line(&mut buf)? == 0 {
        bail!("unexpected end of input");
    }
    Ok(buf.trim().to_owned())
}

fn prompt_optional(prompt: &str) -> Result<Option<String>> {
    let s = read_line(prompt)?;
    Ok(if s.is_empty() { None } else { Some(s) })
}

fn confirm(prompt: &str) -> Result<bool> {
    let s = read_line(prompt)?.to_ascii_lowercase();
    Ok(matches!(s.as_str(), "y" | "yes"))
}

async fn warn_if_super_admin_exists(pool: &SqlitePool) -> Result<()> {
    let count = admin::super_admin_count(pool).await?;
    if count > 0 {
        println!("Note: {count} super admin account(s) already exist.");
        if !confirm("Create another one? [y/N]: ")? {
            println!("Aborted.");
            std::process::exit(0);
        }
    }
    Ok(())
}

async fn prompt_email(pool: &SqlitePool) -> Result<String> {
    loop {
        let email = read_line("Email: ")?;
        if let Err(msg) = admin::validate_email(&email) {
            eprintln!("  ✗ {msg}");
            continue;
        }
        if admin::email_in_use(pool, &email).await? {
            eprintln!("  ✗ That email is already in use.");
            continue;
        }
        return Ok(email);
    }
}

async fn prompt_username(pool: &SqlitePool) -> Result<Option<String>> {
    loop {
        let Some(username) = prompt_optional("Username (optional): ")? else {
            return Ok(None);
        };
        if let Err(msg) = admin::validate_username(&username) {
            eprintln!("  ✗ {msg}");
            continue;
        }
        if admin::username_in_use(pool, &username).await? {
            eprintln!("  ✗ That username is already in use.");
            continue;
        }
        return Ok(Some(username));
    }
}

fn prompt_password() -> Result<SecretString> {
    let (min, max) = admin::password_bounds();
    loop {
        let pw = SecretString::from(rpassword::prompt_password("Password: ")?);
        if let Err(msg) = admin::validate_password(pw.expose_secret()) {
            eprintln!("  ✗ {msg} (allowed length: {min}-{max})");
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
