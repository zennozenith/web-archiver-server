use crate::AppState;

use axum::Router;

pub(crate) fn router(state: AppState) -> Router {
    Router::new().with_state(state)
}
