use std::{path::Path, time::Duration};

use sqlx::{
    Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use crate::{
    config::DatabaseSettings,
    util::time::{days_ms, now_ms},
};

pub(crate) mod api_key;
pub(crate) mod archive;
pub(crate) mod audit;
pub(crate) mod auth;
pub(crate) mod blob;
pub(crate) mod capture;
pub(crate) mod collection;
pub(crate) mod content;
pub(crate) mod create;
pub(crate) mod cursor;
pub(crate) mod group;
pub(crate) mod page;
pub(crate) mod patch;
pub(crate) mod pending;
pub(crate) mod permission;
pub(crate) mod profile;
pub(crate) mod role;
pub(crate) mod session;
pub(crate) mod stats;
pub(crate) mod tag;
pub(crate) mod user;

/// Waiting longer than this for a pooled connection is logged at `WARN`. On the
/// single-connection write pool it means writers are queueing behind a slow one.
const SLOW_ACQUIRE: Duration = Duration::from_millis(500);

/// SQLite pools split by access pattern.
///
/// WAL lets readers run concurrently with the single writer, so mutations go
/// through a one-connection write pool while reads use a larger pool whose
/// connections are `query_only`: a write routed to the read pool by mistake
/// fails loudly instead of silently contending for the write lock.
///
/// There is deliberately no `Deref`. Every call site names the pool it wants
/// via [`Database::reader`], [`Database::writer`] or [`Database::begin_write`].
/// In-memory URLs get one shared pool, because separate connections would see
/// separate databases (and therefore no `query_only` there).
#[derive(Debug, Clone)]
pub(crate) struct Database {
    write: SqlitePool,
    read: SqlitePool,
}

/// What a pool's connections are allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoolMode {
    ReadWrite,
    /// `PRAGMA query_only = ON` is applied to every new connection.
    QueryOnly,
}

impl Database {
    pub(crate) async fn open(settings: &DatabaseSettings) -> Result<Self, sqlx::Error> {
        let options = settings.connect_options()?;
        create_parent_dir(&options).await?;
        let path = options.get_filename().display().to_string();

        let in_memory =
            options.get_filename() == Path::new(":memory:") || settings.url.contains("mode=memory");
        if in_memory {
            // A second connection to `:memory:` would open a different database,
            // so both handles share the same single-connection pool.
            let pool = build_pool(options, 1, PoolMode::ReadWrite).await?;
            return Ok(Self {
                write: pool.clone(),
                read: pool,
            });
        }

        let write = build_pool(options.clone(), 1, PoolMode::ReadWrite).await?;
        let read = build_pool(
            options,
            settings.read_connections.max(1),
            PoolMode::QueryOnly,
        )
        .await?;
        tracing::info!(
            %path,
            read_connections = settings.read_connections,
            slow_query_ms = settings.slow_query_ms,
            "database opened"
        );
        Ok(Self { write, read })
    }

    /// The single-writer pool. Mutations and transactions must use this.
    pub(crate) const fn writer(&self) -> &SqlitePool {
        &self.write
    }

    /// Read-only queries. Writes through this pool fail (`query_only`).
    pub(crate) const fn reader(&self) -> &SqlitePool {
        &self.read
    }

    /// Begin a transaction on the writer. Transactions never start on the
    /// reader, and must not call back into the pool they hold the only
    /// connection of: do any other database work before calling this.
    pub(crate) async fn begin_write(&self) -> Result<Transaction<'static, Sqlite>, sqlx::Error> {
        self.write.begin().await
    }
}

/// A single read-write connection, for tools that do not need the split (the
/// operator CLI).
pub(crate) async fn create_pool(settings: &DatabaseSettings) -> Result<SqlitePool, sqlx::Error> {
    let options = settings.connect_options()?;
    create_parent_dir(&options).await?;
    build_pool(options, 1, PoolMode::ReadWrite).await
}

async fn build_pool(
    options: SqliteConnectOptions,
    max_connections: u32,
    mode: PoolMode,
) -> Result<SqlitePool, sqlx::Error> {
    let mut pool_options = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .acquire_slow_threshold(SLOW_ACQUIRE)
        .idle_timeout(Duration::from_secs(10 * 60))
        .max_lifetime(Duration::from_secs(30 * 60));

    if mode == PoolMode::QueryOnly {
        // Runs after the connect-time PRAGMAs (WAL, foreign keys, ...), so the
        // journal mode can still be established before writes are disabled.
        pool_options = pool_options.after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("PRAGMA query_only = ON")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        });
    }

    pool_options.connect_with(options).await
}

/// `create_if_missing` creates the file, not its parent directory.
async fn create_parent_dir(options: &SqliteConnectOptions) -> Result<(), sqlx::Error> {
    if let Some(parent) = parent_dir(options) {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(sqlx::Error::Io)?;
    }
    Ok(())
}

/// Parent directory of the database file, unless this is an in-memory URL.
fn parent_dir(options: &SqliteConnectOptions) -> Option<&Path> {
    let filename = options.get_filename();
    if filename == Path::new(":memory:") {
        return None;
    }
    filename.parent().filter(|p| !p.as_os_str().is_empty())
}

/// Apply all migrations in ./migrations (embedded into the binary at compile time).
pub(crate) async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    let migrator = sqlx::migrate!("./migrations");
    tracing::info!(
        bundled = migrator.iter().count(),
        "applying database migrations"
    );
    migrator.run(pool).await?;
    tracing::info!("database schema is up to date");
    Ok(())
}

/// Hourly housekeeping, taken from the "OPERATIONAL NOTES" in the schema file.
///
/// The returned handle stops when `shutdown` is cancelled.
pub(crate) fn spawn_cleanup(pool: SqlitePool, shutdown: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                _ = tick.tick() => {}
            }
            // `instrument` (not `span.enter()`): a guard held across `.await`
            // would attach the span to whatever else runs on this thread.
            cleanup_once(&pool)
                .instrument(tracing::info_span!("db.cleanup"))
                .await;
        }
    })
}

async fn cleanup_once(pool: &SqlitePool) {
    let now = now_ms();
    let cutoff = |days: i64| now.saturating_sub(days_ms(days));

    let jobs = [
        ("DELETE FROM sessions WHERE expires_at < ?", cutoff(30)),
        ("DELETE FROM auth_tokens WHERE expires_at < ?", cutoff(7)),
        (
            "DELETE FROM login_attempts WHERE created_at < ?",
            cutoff(90),
        ),
    ];
    for (sql, ts) in jobs {
        run_cleanup(pool, sql, ts).await;
    }
    let _throw_away = sqlx::query("PRAGMA optimize").execute(pool).await;
}

async fn run_cleanup(pool: &SqlitePool, sql: &'static str, ts: i64) {
    match sqlx::query(sql).bind(ts).execute(pool).await {
        Ok(r) => {
            let removed = r.rows_affected();
            if removed > 0 {
                tracing::info!(removed, sql, "cleanup removed expired rows");
            }
        }
        Err(error) => tracing::warn!(%error, sql, "cleanup failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn all_migrations_apply_cleanly() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        migrate(&pool).await.expect("migrations apply");

        let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .expect("migration table");
        assert!(applied > 0, "expected at least one applied migration");
    }

    #[tokio::test]
    async fn reader_is_query_only_and_sees_committed_writes() {
        let path = std::env::temp_dir().join(format!(
            "web-archiver-db-test-{}-{}.db",
            std::process::id(),
            now_ms()
        ));
        let settings = DatabaseSettings {
            url: format!("sqlite://{}", path.display()).into(),
            ..DatabaseSettings::default()
        };
        let db = Database::open(&settings).await.expect("open database");

        sqlx::query("CREATE TABLE t (v INTEGER NOT NULL)")
            .execute(db.writer())
            .await
            .expect("create table on writer");
        sqlx::query("INSERT INTO t (v) VALUES (1)")
            .execute(db.writer())
            .await
            .expect("insert on writer");

        let seen: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM t")
            .fetch_one(db.reader())
            .await
            .expect("reader sees committed rows");
        assert_eq!(seen, 1);

        let write = sqlx::query("INSERT INTO t (v) VALUES (2)")
            .execute(db.reader())
            .await;
        assert!(write.is_err(), "the read pool must reject writes");

        db.writer().close().await;
        db.reader().close().await;
        for suffix in ["", "-wal", "-shm"] {
            drop(tokio::fs::remove_file(format!("{}{suffix}", path.display())).await);
        }
    }
}
