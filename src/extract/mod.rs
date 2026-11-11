#![expect(
    dead_code,
    reason = "alternative extractors and deserializers are kept for future routes"
)]

use crate::error::AppError;

use axum::{
    Form, Json, RequestExt,
    extract::{
        FromRequest, FromRequestParts, Request,
        rejection::{FormRejection, JsonRejection},
    },
};
use http::header::CONTENT_TYPE;
use serde::Deserialize;

pub(crate) mod auth_user;

/// Query string; supports repeated keys (?tag=a&tag=b).
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(AppError))]
pub(crate) struct ApiQuery<T>(pub T);

/// Path params; bad ids return the API error shape instead of plain text.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(AppError))]
pub(crate) struct ApiPath<T>(pub T);

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
pub(crate) struct ApiJson<T>(pub T);

impl<T: serde::Serialize> axum::response::IntoResponse for ApiJson<T> {
    fn into_response(self) -> axum::response::Response {
        axum::Json(self.0).into_response()
    }
}

#[derive(FromRequest)]
#[from_request(via(axum::Form), rejection(AppError))]
pub(crate) struct ApiForm<T>(pub T);

impl<T: serde::Serialize> axum::response::IntoResponse for ApiForm<T> {
    fn into_response(self) -> axum::response::Response {
        axum::Json(self.0).into_response()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct JsonOrForm<T>(pub T);
pub(crate) type FormOrJson<T> = JsonOrForm<T>;

impl<S, T> FromRequest<S> for JsonOrForm<T>
where
    S: Send + Sync,
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    Form<T>: FromRequest<S, Rejection = FormRejection>,
    T: for<'de> Deserialize<'de> + 'static,
{
    type Rejection = AppError;

    async fn from_request(req: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let content_type = req
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_ascii_lowercase);

        match content_type.as_deref() {
            Some(ct) if ct.starts_with("application/json") => {
                let Json(payload) = req.extract().await?;
                Ok(Self(payload))
            }
            Some(ct) if ct.starts_with("application/x-www-form-urlencoded") => {
                let Form(payload) = req.extract().await?;
                Ok(Self(payload))
            }
            _ => Err(AppError::UnsupportedMediaType),
        }
    }
}

// ---------------------------

use serde::de::{self, Deserializer, SeqAccess, Visitor};
use std::fmt;

pub(crate) fn comma_separated_or_seq<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringOrVec;

    impl<'de> Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a comma-separated string or an array of strings")
        }

        // form input: "a,b,c"
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect())
        }

        // json input: ["a","b","c"]
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(item) = seq.next_element::<String>()? {
                out.push(item);
            }
            Ok(out)
        }
    }

    deserializer.deserialize_any(StringOrVec)
}

pub(crate) fn opt_comma_separated_or_seq<'de, D>(d: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    comma_separated_or_seq(d).map(Some)
}
