use axum::extract::FromRequestParts;
use http::request::Parts;

use crate::{auth::ClientInfo, error::AppError};

impl<S: Send + Sync> FromRequestParts<S> for ClientInfo {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Self::extract(parts))
    }
}
