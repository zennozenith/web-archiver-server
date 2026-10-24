use axum::{Json, extract::State};

use crate::{AppState, db, dto::HealthDto, error::AppError};

pub async fn health(State(st): State<AppState>) -> Result<Json<HealthDto>, AppError> {
    Ok(Json(db::health::health(&st.db).await?))
}
