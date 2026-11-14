use std::{io, net::AddrParseError};

use axum::{
    Json,
    extract::{
        multipart::{MultipartError, MultipartRejection},
        rejection::{FormRejection, JsonRejection, PathRejection, QueryRejection},
    },
    response::{IntoResponse, Response},
};
use http::{StatusCode, header::InvalidHeaderValue};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    error::AppError::{
        Forbidden, HashParse, InvalidCredentials, TooManyRequests, UnsupportedMediaType,
    },
    infra::HashParseError,
};

pub(crate) type Result<T, E = AppError> = std::result::Result<T, E>;

#[derive(Clone)]
pub(crate) struct ErrorContext {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorResponse<'a> {
    error: ErrorPayload<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorPayload<'a> {
    code: &'a str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
}

/// Typed description of the resource a `NotFound` refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    Other,
    Page,
    Capture,
    CaptureContent,
    PendingItem,
    User,
    Role,
    UserOrRole,
    Collection,
    PublicCollection,
    Tag,
    PageTag,
    Highlight,
    ApiKey,
    Session,
    Content,
    Asset,
    Thumbnail,
}

impl std::fmt::Display for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Other => "resource",
            Self::Page => "page",
            Self::Capture => "capture",
            Self::CaptureContent => "capture content",
            Self::PendingItem => "pending item",
            Self::User => "user",
            Self::Role => "role",
            Self::UserOrRole => "user or role",
            Self::Collection => "collection",
            Self::PublicCollection => "public collection",
            Self::Tag => "tag",
            Self::PageTag => "tag on page",
            Self::Highlight => "highlight",
            Self::ApiKey => "api_key",
            Self::Session => "session",
            Self::Content => "content",
            Self::Asset => "asset",
            Self::Thumbnail => "thumbnail",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    // 400
    #[error("invalid url: {0}")]
    InvalidUrl(String),
    #[error("Failed to parse hash: {0}")]
    HashParse(#[from] HashParseError),
    #[error("{0}")]
    BadRequest(String),

    // 401
    #[error("authentication required")]
    Unauthorized,
    #[error("invalid credentials")]
    InvalidCredentials,

    // 403
    #[error("Forbidden: {0}")]
    Forbidden(#[from] ForbiddenReason),

    // 404
    #[error("{0} not found")]
    NotFound(Resource),

    // 409
    #[error("{msg}. Existing id: {existing_id}")]
    Duplicate { existing_id: i64, msg: &'static str },
    #[error("{0}")]
    Conflict(String),

    // 413
    #[error("payload{} too large", limit.map_or_else(String::new, |limit| format!(" exceeds {limit} bytes")))]
    PayloadTooLarge { limit: Option<usize> },

    // 415
    #[error("unsupported media type")]
    UnsupportedMediaType,

    // 429
    #[error("{0}")]
    TooManyRequests(&'static str),

    // 422
    #[error("{message}")]
    Invalid { code: &'static str, message: String },

    // 500
    #[error("Failed to render template. Error: {0}")]
    Render(#[from] askama::Error),
    #[error("database error")]
    Database(#[source] sqlx::Error),
    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl AppError {
    pub(crate) fn parts(&self) -> (StatusCode, &'static str, Option<Value>) {
        use AppError::{
            BadRequest, Conflict, Database, Duplicate, Internal, Invalid, InvalidUrl, NotFound,
            PayloadTooLarge, Render, Unauthorized,
        };
        match self {
            InvalidUrl(_) => (StatusCode::BAD_REQUEST, "invalid_url", None),
            HashParse(_) => (StatusCode::BAD_REQUEST, "bad_hash", None),
            BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request", None),
            Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", None),
            InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials", None),
            Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden", None),
            NotFound(_) => (StatusCode::NOT_FOUND, "not_found", None),
            Duplicate { existing_id, msg } => (
                StatusCode::CONFLICT,
                "duplicate",
                Some(json!({ "existingId": existing_id, "message": msg })),
            ),
            Conflict(_) => (StatusCode::CONFLICT, "conflict", None),
            PayloadTooLarge { limit } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                Some(json!({ "limit": limit })),
            ),
            UnsupportedMediaType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                None,
            ),
            TooManyRequests(_) => (StatusCode::TOO_MANY_REQUESTS, "too_many_requests", None),
            Invalid { code, .. } => (StatusCode::UNPROCESSABLE_ENTITY, code, None),
            Database(_) | Internal(_) | Render(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
            }
        }
    }

    pub(crate) fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self::Invalid {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn internal(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Internal(error.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ForbiddenReason {
    #[error("please verify your e-mail address first")]
    EmailUnverified,
    #[error("this account is suspended")]
    Suspended,
    #[error("this account is deactivated")]
    Deactivated,
    #[error("you do not have permission")]
    MissingPermission,
    #[error("keys cannot mint more keys")]
    ApiKeyMint,
    #[error("request must come from the app")]
    Csrf,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, details) = self.parts();

        // Log server errors with the full cause chain, but never leak it to clients.
        let message = if status.is_server_error() {
            tracing::error!(error = ?self, %status, "request failed");
            "internal server error".to_owned()
        } else {
            self.to_string()
        };

        let info = ErrorContext {
            status,
            code,
            message,
            details,
        };

        // Default representation: JSON
        let mut res = (
            status,
            Json(ErrorResponse {
                error: ErrorPayload {
                    code: info.code,
                    message: info.message.clone(),
                    details: info.details.clone(),
                },
            }),
        )
            .into_response();

        // Let the negotiation middleware re-render it if the client wants HTML.
        res.extensions_mut().insert(info);
        res
    }
}

// ── Conversions so `?` just works ───────────────────────────────────────────

impl From<sqlx::Error> for AppError {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "sqlx::Error is non-exhaustive; all unhandled variants become database errors"
    )]
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::RowNotFound => Self::NotFound(Resource::Other),
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                Self::Conflict("resource already exists".into())
            }
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => Self::Invalid {
                code: "invalid_reference",
                message: "referenced resource does not exist".into(),
            },
            _ => Self::Database(e),
        }
    }
}

impl From<io::Error> for AppError {
    fn from(e: io::Error) -> Self {
        Self::internal(e)
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::internal(e)
    }
}

impl From<getrandom::Error> for AppError {
    fn from(e: getrandom::Error) -> Self {
        Self::internal(e)
    }
}

impl From<InvalidHeaderValue> for AppError {
    fn from(e: InvalidHeaderValue) -> Self {
        Self::internal(e)
    }
}

impl From<AddrParseError> for AppError {
    fn from(e: AddrParseError) -> Self {
        Self::internal(e)
    }
}

impl From<MultipartError> for AppError {
    fn from(e: MultipartError) -> Self {
        from_rejection(e.status(), e.body_text())
    }
}

/// Map any axum rejection from its status and client-safe body text.
fn from_rejection(status: StatusCode, message: String) -> AppError {
    match status {
        StatusCode::UNSUPPORTED_MEDIA_TYPE => UnsupportedMediaType,
        StatusCode::PAYLOAD_TOO_LARGE => AppError::PayloadTooLarge { limit: None },
        StatusCode::UNPROCESSABLE_ENTITY => AppError::invalid("invalid_body", message),
        _ => AppError::BadRequest(message),
    }
}

impl From<PathRejection> for AppError {
    fn from(r: PathRejection) -> Self {
        Self::BadRequest(r.body_text())
    }
}

impl From<QueryRejection> for AppError {
    fn from(r: QueryRejection) -> Self {
        Self::BadRequest(r.body_text())
    }
}

impl From<JsonRejection> for AppError {
    fn from(e: JsonRejection) -> Self {
        from_rejection(e.status(), e.body_text())
    }
}

impl From<FormRejection> for AppError {
    fn from(e: FormRejection) -> Self {
        from_rejection(e.status(), e.body_text())
    }
}

impl From<MultipartRejection> for AppError {
    fn from(r: MultipartRejection) -> Self {
        from_rejection(r.status(), r.body_text())
    }
}
