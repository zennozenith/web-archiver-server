use sqlx::{SqliteConnection, SqlitePool};

use crate::{error::AppError, models::TagRow, tag::CleanTag, url_canon::CanonicalUrl};

// TODO: Should functions be only public to super

pub(crate) async fn missing_pending_or(
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

pub(crate) async fn replace_pending_tags(
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

pub(crate) async fn get_or_create_tag(
    conn: &mut SqliteConnection,
    name: &CleanTag,
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

/// Union tags into a page (never removes existing ones).
pub(crate) async fn add_url_tags(
    conn: &mut SqliteConnection,
    url_id: i64,
    names: &[CleanTag],
) -> Result<(), sqlx::Error> {
    for name in names {
        let tag_id = get_or_create_tag(&mut *conn, name).await?;
        sqlx::query("INSERT OR IGNORE INTO url_tag (url_id, tag_id) VALUES (?, ?)")
            .bind(url_id)
            .bind(tag_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

pub(crate) async fn get_or_create_url(
    conn: &mut SqliteConnection,
    canon: &CanonicalUrl,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO url (url, scheme, host, path, query) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(url) DO UPDATE SET url = url
         RETURNING id",
    )
    .bind(&canon.url)
    .bind(canon.scheme.as_str())
    .bind(&canon.host)
    .bind(&canon.path)
    .bind(&canon.query)
    .fetch_one(&mut *conn)
    .await
}

pub(crate) async fn load_tag(
    conn: &mut SqliteConnection,
    id: i64,
) -> Result<Option<TagRow>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, color, created_at FROM tag WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
}

pub(crate) async fn replace_url_tags(
    conn: &mut SqliteConnection,
    url_id: i64,
    names: &[CleanTag],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM url_tag WHERE url_id = ?")
        .bind(url_id)
        .execute(&mut *conn)
        .await?;
    add_url_tags(conn, url_id, names).await
}

pub(crate) async fn page_tags(
    conn: &mut SqliteConnection,
    url_id: i64,
) -> Result<Vec<TagRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT t.id, t.name, t.color, t.created_at
         FROM url_tag ut JOIN tag t ON t.id = ut.tag_id
         WHERE ut.url_id = ? ORDER BY t.name COLLATE NOCASE",
    )
    .bind(url_id)
    .fetch_all(&mut *conn)
    .await
}
