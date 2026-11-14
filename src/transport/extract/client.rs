use axum::extract::{FromRef, FromRequestParts};
use http::request::Parts;

use crate::{app::AppState, error::AppError, identity::ClientInfo};

impl<S> FromRequestParts<S> for ClientInfo
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Self::extract(parts, &AppState::from_ref(state).server))
    }
}
