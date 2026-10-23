use axum::{Json, extract::State};

use crate::{AppState, dto::HealthDto, error::Result};

pub async fn health(State(st): State<AppState>) -> Result<Json<HealthDto>> {
    let (urls, versions, contents, pending): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(DISTINCT url_id) FROM capture),
                (SELECT COUNT(*) FROM capture),
                (SELECT COUNT(*) FROM content),
                (SELECT COUNT(*) FROM pending)",
    )
    .fetch_one(&st.db)
    .await?;

    Ok(Json(HealthDto {
        status: "online",
        urls,
        versions,
        contents,
        pending,
    }))
}
