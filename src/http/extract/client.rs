use axum::extract::{FromRef, FromRequestParts};
use http::request::Parts;

use crate::{app::AppState, auth::ClientInfo, error::AppError};

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
        let trusted_proxy = AppState::from_ref(state).trusted_proxy;
        std::future::ready(Self::extract(parts, trusted_proxy))
    }
}
