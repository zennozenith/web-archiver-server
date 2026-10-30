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

// pub trait IntoAnyhow<T> {
//     fn anyhow(self) -> anyhow::Result<T>;
// }

// impl<T, E> IntoAnyhow<T> for Result<T, E>
// where
//     E: std::error::Error + Send + Sync + 'static,
// {
//     fn anyhow(self) -> anyhow::Result<T> {
//         self.map_err(anyhow::Error::from)
//     }
// }

pub(crate) type Result<T, E = AppError> = std::result::Result<T, E>;

#[derive(Clone)]
pub(crate) struct ErrorInfo {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorInner<'a>,
}

#[derive(Serialize)]
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
    #[error("{0}")]
    BadRequest(String),

    // 401
    #[error("missing or invalid credentials")]
    Unauthorized,

    // 404
    #[error("{0} not found")]
    NotFound(&'static str),

    // 409
    #[error("url is already in the pending queue")]
    DuplicatePending { existing_id: i64 },
    #[error("a tag with that name already exists")]
    DuplicateTag { existing_id: i64 },

    #[error("{0}")]
    Conflict(String),

    // 413
    #[error("payload exceeds {limit} bytes")]
    PayloadTooLarge { limit: usize },

    // 415
    #[error("unsupported media type")]
    UnsupportedMediaType,

    // // pass-through for any other axum rejection (e.g. 413 from body limits)
    // #[error("{message}")]
    // Rejection { status: StatusCode, message: String },

    // 422
    #[error("{message}")]
    Unprocessable { code: &'static str, message: String },

    // 500
    #[error("Failed to render template. Error: {0}")]
    Render(
        #[source]
        #[from]
        askama::Error,
    ),
    #[error("database error")]
    Database(#[source] sqlx::Error),
    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl AppError {
    fn parts(&self) -> (StatusCode, &'static str, Option<Value>) {
        use AppError::{
            BadRequest, Conflict, Database, DuplicatePending, DuplicateTag, Internal, InvalidUrl,
            NotFound, PayloadTooLarge, Render, Unauthorized, Unprocessable,
        };
        match self {
            InvalidUrl(_) => (StatusCode::BAD_REQUEST, "invalid_url", None),
            BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request", None),
            Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", None),
            NotFound(_) => (StatusCode::NOT_FOUND, "not_found", None),
            DuplicatePending { existing_id } => (
                StatusCode::CONFLICT,
                "duplicate_pending",
                Some(json!({ "existing_id": existing_id })),
            ),
            DuplicateTag { existing_id } => (
                StatusCode::CONFLICT,
                "duplicate_tag",
                Some(json!({ "existing_id": existing_id })),
            ),
            Conflict(_) => (StatusCode::CONFLICT, "conflict", None),
            PayloadTooLarge { limit } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                Some(json!({ "limit": limit })),
            ),
            Self::UnsupportedMediaType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported media type",
                None,
            ),
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
