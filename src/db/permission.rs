use sqlx::{FromRow, SqlitePool};

#[derive(Debug, FromRow)]
pub(crate) struct PermissionInfo {
    pub name: String,
    pub description: Option<String>,
}

pub(crate) async fn all(pool: &SqlitePool) -> sqlx::Result<Vec<PermissionInfo>> {
    sqlx::query_as("SELECT name, description FROM permissions ORDER BY id")
        .fetch_all(pool)
        .await
}
