use sqlx::SqlitePool;

use crate::dto::HealthDto;

pub async fn health(pool: &SqlitePool) -> Result<HealthDto, sqlx::Error> {
    let (urls, tags, versions, contents, pending): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(DISTINCT url_id) FROM capture),
                (SELECT COUNT(*) FROM tag),
                (SELECT COUNT(*) FROM capture),
                (SELECT COUNT(*) FROM content),
                (SELECT COUNT(*) FROM pending)",
    )
    .fetch_one(pool)
    .await?;

    Ok(HealthDto {
        status: "online",
        tags,
        urls,
        versions,
        contents,
        pending,
    })
}
