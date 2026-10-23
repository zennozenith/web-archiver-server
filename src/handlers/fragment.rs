use axum::{Router, routing::post};

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/pending", post(pending::create_pending))
}

mod pending {
    use axum::{Form, extract::State, http::StatusCode, response::Html};
    use sqlx::SqlitePool;

    use crate::{
        db,
        dto::{CreatePending, PendingDto},
        error::Result,
        extract::ApiJson,
    };

    pub async fn create_pending(
        State(pool): State<SqlitePool>,
        Form(req): Form<CreatePending>,
    ) -> Result<(StatusCode, Html<PendingDto>)> {
        Ok((
            StatusCode::CREATED,
            ApiJson(db::pending::create_pending(&pool, req).await?),
        ))
    }
}
