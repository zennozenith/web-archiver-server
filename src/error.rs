use axum::{
    Json,
    extract::{
        multipart::{MultipartError, MultipartRejection},
        rejection::{FormRejection, JsonRejection, PathRejection, QueryRejection},
    },
    response::{IntoResponse, Response},
};
use http::StatusCode;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    error::AppError::{
        Forbidden, HashParse, InvalidCredentials, TooManyRequest, UnsupportedMediaType,
    },
    infra::HashParseError,
};

pub(crate) type Result<T, E = AppError> = std::result::Result<T, E>;

#[derive(Clone)]
pub(crate) struct ErrorInfo {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody<'a> {
    error: ErrorInner<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorInner<'a> {
    code: &'a str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
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
    #[error("you do not have permission: {0}")]
    Forbidden(#[from] ForbiddenReason),

    // 404
    #[error("{0} not found")]
    NotFound(&'static str),

    // 409
    #[error("{msg}. Existing id: {existing_id}")]
    Duplicate { existing_id: i64, msg: &'static str },
    #[error("{0}")]
    Conflict(String),

    // 413
    #[error("payload exceeds {limit} bytes")]
    PayloadTooLarge { limit: usize },

    // 415
    #[error("unsupported media type")]
    UnsupportedMediaType,

    // 429
    #[error("{0}")]
    TooManyRequest(&'static str),

    // // pass-through for any other axum rejection (e.g. 413 from body limits)
    // #[error("{message}")]
    // Rejection { status: StatusCode, message: String },

    // 422
    #[error("{message}")]
    Unprocessable { code: &'static str, message: String },

    // 500
    #[error("Failed to render template. Error: {0}")]
    Render(#[from] askama::Error),
    #[error("database error")]
    Database(sqlx::Error),
    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl AppError {
    pub(crate) fn parts(&self) -> (StatusCode, &'static str, Option<Value>) {
        use AppError::{
            BadRequest, Conflict, Database, Duplicate, Internal, InvalidUrl, NotFound,
            PayloadTooLarge, Render, Unauthorized, Unprocessable,
        };
        match self {
            InvalidUrl(_) => (StatusCode::BAD_REQUEST, "invalid_url", None),
            HashParse(_) => (StatusCode::BAD_REQUEST, "bad_hash", None),
            BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request", None),
            Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", None),
            InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials", None),
            Forbidden(e) => {
                if *e == ForbiddenReason::Unknown {
                    tracing::error!("New field for forbidden reason");
                }
                (StatusCode::FORBIDDEN, "forbidden", None)
            }
            NotFound(_) => (StatusCode::NOT_FOUND, "not_found", None),
            Duplicate { existing_id, msg } => (
                StatusCode::CONFLICT,
                "duplicate",
                Some(json!({ "existingId": existing_id, "error": msg })),
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
            TooManyRequest(_) => (StatusCode::TOO_MANY_REQUESTS, "too_many_request", None),
            Unprocessable { code, .. } => (StatusCode::UNPROCESSABLE_ENTITY, code, None),
            Database(_) | Internal(_) | Render(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
            }
        }
    }

    pub fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self::Unprocessable {
            code,
            message: message.into(),
        }
    }

    /// Message safe to show a client. Server errors are logged with their full
    /// cause and replaced with a generic message, so internals never leak.
    pub(crate) fn public_message(&self, status: StatusCode) -> String {
        if status.is_server_error() {
            tracing::error!(error = ?self, %status, "request failed");
            "internal server error".to_owned()
        } else {
            self.to_string()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ForbiddenReason {
    #[error("please verify your e-mail address first")]
    Pending,
    #[error("this account is suspended")]
    Suspended,
    #[error("this account is deactivated")]
    Deactivated,
    #[error("you do not have permission")]
    Permission,
    #[error("keys cannot mint more keys")]
    ApiKeyMint,
    #[error("Csrf")]
    Csrf,
    #[error("unknown reason, report admin")]
    Unknown,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, details) = self.parts();

        // Log server errors with the full cause chain, but never leak it to clients.
        let message = if status.is_server_error() {
            tracing::error!(error = ?self, "request failed");
            "internal server error".to_owned()
        } else {
            self.to_string()
        };

        let info = ErrorInfo {
            status,
            code,
            message,
            details,
        };

        // Default representation: JSON
        let mut res = (
            status,
            Json(ErrorBody {
                error: ErrorInner {
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
            sqlx::Error::RowNotFound => Self::NotFound("resource"),
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                Self::Conflict("resource already exists".into())
            }
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => Self::Unprocessable {
                code: "invalid_reference",
                message: "referenced resource does not exist".into(),
            },
            _ => Self::Database(e),
        }
    }
}

impl From<MultipartError> for AppError {
    fn from(e: MultipartError) -> Self {
        match e.status() {
            StatusCode::PAYLOAD_TOO_LARGE => Self::PayloadTooLarge { limit: 0 },
            _ => Self::BadRequest(format!("invalid multipart body: {}", e.body_text())),
        }
    }
}

fn from_rejection(status: StatusCode, message: String) -> AppError {
    match status {
        StatusCode::UNSUPPORTED_MEDIA_TYPE => AppError::UnsupportedMediaType,
        StatusCode::PAYLOAD_TOO_LARGE => AppError::PayloadTooLarge { limit: 0 },
        StatusCode::UNPROCESSABLE_ENTITY => AppError::Unprocessable {
            code: "invalid_body",
            message,
        },
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
        match r.status() {
            StatusCode::PAYLOAD_TOO_LARGE => Self::PayloadTooLarge { limit: 0 },
            _ => Self::BadRequest(r.body_text()),
        }
    }
}

/// Extension to turn `Option` into `NotFound` ergonomically:
/// `row.or_not_found("page")?`
pub(crate) trait OptionExt<T> {
    fn or_not_found(self, what: &'static str) -> Result<T>;
}
impl<T> OptionExt<T> for Option<T> {
    fn or_not_found(self, what: &'static str) -> Result<T> {
        self.ok_or(AppError::NotFound(what))
    }
}

pub(crate) fn join_err(e: tokio::task::JoinError) -> AppError {
    AppError::Internal(Box::new(e))
}
