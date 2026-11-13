use axum::extract::{FromRef, FromRequestParts};
use http::{HeaderMap, header, request::Parts};
use sqlx::SqlitePool;

use crate::{
    auth::{ApiKeyToken, AuthUser, Credential, SessionToken},
    error::AppError,
    http::cookie::{SessionCookie, cookie_value},
    service,
};

fn credential(headers: &HeaderMap) -> Option<Credential> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);

    if let Some(cred) = bearer
        .and_then(|key| ApiKeyToken::try_from(key.to_owned()).ok())
        .map(Credential::ApiKey)
    {
        return Some(cred);
    }

    cookie_value(headers, SessionCookie::KEY)
        .and_then(|t| SessionToken::try_from(t).ok())
        .map(Credential::Session)
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    SqlitePool: FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let cred = credential(&parts.headers).ok_or(AppError::Unauthorized)?;
        service::auth::authenticate(&SqlitePool::from_ref(state), cred).await
    }
}
