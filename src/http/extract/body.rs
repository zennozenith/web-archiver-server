use axum::{
    Form, Json, RequestExt,
    extract::{
        FromRequest, FromRequestParts, Request,
        rejection::{FormRejection, JsonRejection},
    },
    response::{IntoResponse, Response},
};
use http::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(AppError))]
pub(crate) struct AppQuery<T>(pub T);

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(AppError))]
pub(crate) struct AppPath<T>(pub T);

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
pub(crate) struct AppJson<T>(pub T);

impl<T: Serialize> IntoResponse for AppJson<T> {
    fn into_response(self) -> Response {
        Json(self.0).into_response()
    }
}

#[derive(FromRequest)]
#[from_request(via(axum::Form), rejection(AppError))]
pub(crate) struct AppForm<T>(pub T);

/// Accepts a JSON or urlencoded body into the same DTO, so one request
/// struct can serve both the API and the htmx form.
pub(crate) struct JsonOrForm<T>(pub T);

impl<S, T> FromRequest<S> for JsonOrForm<T>
where
    S: Send + Sync,
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    Form<T>: FromRequest<S, Rejection = FormRejection>,
    T: for<'de> Deserialize<'de> + 'static,
{
    type Rejection = AppError;

    async fn from_request(req: Request, _: &S) -> Result<Self, Self::Rejection> {
        let ct = req
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map_or_else(String::new, str::to_ascii_lowercase);

        if ct.starts_with("application/json") {
            let Json(v) = req.extract().await?;
            Ok(Self(v))
        } else if ct.starts_with("application/x-www-form-urlencoded") {
            let Form(v) = req.extract().await?;
            Ok(Self(v))
        } else {
            Err(AppError::UnsupportedMediaType)
        }
    }
}
