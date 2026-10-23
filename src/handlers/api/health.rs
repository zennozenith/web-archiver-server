use axum::{Json, extract::State};

use crate::{AppState, db, dto::HealthDto, error::Result};

pub async fn health(State(st): State<AppState>) -> Result<Json<HealthDto>> {
    Ok(Json(db::health::health(&st.db).await?))
}
