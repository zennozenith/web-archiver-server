// use axum::{
//     extract::{FromRef, FromRequestParts},
//     http::{Method, request::Parts},
// };
// use sqlx::SqlitePool;

// use crate::{
//     auth::AuthUser,
//     error::{AppError, ForbiddenReason},
//     http::htmx::HtmxRequest,
// };

// /// A signed-in user for HTML routes.
// ///
// /// State-changing requests must come from htmx. A cross-site HTML form can't
// /// set `HX-Request`, which complements `SameSite=Lax` as CSRF protection.
// #[derive(Debug)]
// pub(crate) struct AuthUser(pub(crate) AuthUser);

// impl<S> FromRequestParts<S> for AuthUser
// where
//     S: Send + Sync,
//     SqlitePool: FromRef<S>,
// {
//     type Rejection = AppError;

//     async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
//         AuthUser::from_request_parts(parts, state).await.map(Self)
//     }
// }
