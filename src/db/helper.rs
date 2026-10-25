use sqlx::{SqliteConnection, SqlitePool};

use crate::{error::AppError, tag::CleanTag};

pub async fn missing_pending_or(
    pool: &SqlitePool,
    id: i64,
    conflict_msg: &str,
) -> Result<AppError, AppError> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM pending WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    Ok(match exists {
        Some(_) => AppError::Conflict(conflict_msg.into()),
        None => AppError::NotFound("pending item"),
    })
}

pub(super) async fn replace_pending_tags(
    conn: &mut SqliteConnection,
    pending_id: i64,
    names: &[CleanTag],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM pending_tag WHERE pending_id = ?")
        .bind(pending_id)
        .execute(&mut *conn)
        .await?;
    for name in names {
        let tag_id = get_or_create_tag(&mut *conn, name).await?;
        sqlx::query("INSERT OR IGNORE INTO pending_tag (pending_id, tag_id) VALUES (?, ?)")
            .bind(pending_id)
            .bind(tag_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

pub(super) async fn get_or_create_tag(
    conn: &mut SqliteConnection,
    name: &str,
) -> Result<i64, sqlx::Error> {
    // DO UPDATE (no-op) so RETURNING yields the existing row, keeping its original casing.
    sqlx::query_scalar(
        "INSERT INTO tag (name) VALUES (?)
         ON CONFLICT(name) DO UPDATE SET name = name
         RETURNING id",
    )
    .bind(name)
    .fetch_one(&mut *conn)
    .await
}
