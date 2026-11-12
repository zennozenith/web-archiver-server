//! Shared plumbing for the HTML routes: the session extractor, error
//! responses, the template renderer and the multi-fragment response builder.

use askama::Template;
use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, HeaderValue, Method, StatusCode, header::HeaderName, request::Parts},
    response::{Html, IntoResponse, Redirect, Response},
};

use crate::{error::ForbiddenReason, extract::auth_user::AuthUser, prelude::*};

const HX_REQUEST: &str = "hx-request";

// ------------------------------------------------------------------ rendering

/// Renders an askama template as an HTML response.
#[derive(Debug)]
pub(crate) struct Render<T>(pub(crate) T);

impl<T: Template> IntoResponse for Render<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(e) => {
                tracing::error!("template render failed: {e}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ToastKind {
    Success,
    Error,
}

impl ToastKind {
    const fn class(self) -> &'static str {
        match self {
            Self::Success => "alert-success",
            Self::Error => "alert-error",
        }
    }
}

#[derive(Debug, Template)]
#[template(path = "partials/toast.templ.html")]
struct ToastTpl {
    kind: &'static str,
    message: String,
}

/// Builds one response out of several rendered templates, e.g. the main
/// fragment + out-of-band updates + a toast. Order matters: main content first.
#[derive(Debug, Default)]
pub(crate) struct Fragments {
    body: String,
    headers: HeaderMap,
    status: Option<StatusCode>,
}

impl Fragments {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push<T: Template>(mut self, tpl: &T) -> Result<Self, AppError> {
        let html = tpl.render().map_err(|e| AppError::Internal(Box::new(e)))?;
        self.body.push_str(&html);
        Ok(self)
    }

    /// Appends a toast (out-of-band, lands in `#toasts`).
    pub(crate) fn toast(self, kind: ToastKind, message: &str) -> Result<Self, AppError> {
        self.push(&ToastTpl {
            kind: kind.class(),
            message: message.to_owned(),
        })
    }

    /// Adds a response header. `name` must be lowercase.
    pub(crate) fn header(mut self, name: &'static str, value: &'static str) -> Self {
        self.headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
        self
    }

    pub(crate) const fn status(mut self, status: StatusCode) -> Self {
        self.status = Some(status);
        self
    }
}

impl IntoResponse for Fragments {
    fn into_response(self) -> Response {
        let status = self.status.unwrap_or(StatusCode::OK);
        (status, self.headers, Html(self.body)).into_response()
    }
}

/// Tell htmx to navigate (full page load) after a successful request.
pub(crate) fn with_redirect(mut resp: Response, to: &'static str) -> Response {
    resp.headers_mut().insert(
        HeaderName::from_static("hx-redirect"),
        HeaderValue::from_static(to),
    );
    resp
}

// --------------------------------------------------------------------- errors

#[derive(Debug, Template)]
#[template(path = "error.templ.html")]
struct ErrorTpl {
    title: &'static str,
    code: u16,
    message: String,
}

/// Error for full-page routes: renders a standalone error page.
#[derive(Debug)]
pub(crate) struct PageError(AppError);

impl From<AppError> for PageError {
    fn from(e: AppError) -> Self {
        Self(e)
    }
}

impl From<sqlx::Error> for PageError {
    fn from(e: sqlx::Error) -> Self {
        Self(AppError::Database(e))
    }
}

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        let (status, ..) = self.0.parts();
        let page = ErrorTpl {
            title: "Something went wrong",
            code: status.as_u16(),
            message: self.0.to_string(),
        };
        (status, Render(page)).into_response()
    }
}

/// Error for htmx action/fragment routes: answers with an error toast and
/// tells htmx not to touch the main target.
#[derive(Debug)]
pub(crate) struct ActionError(AppError);

impl From<AppError> for ActionError {
    fn from(e: AppError) -> Self {
        Self(e)
    }
}

impl From<sqlx::Error> for ActionError {
    fn from(e: sqlx::Error) -> Self {
        Self(AppError::Database(e))
    }
}

fn unauthorized_redirect() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            HeaderName::from_static("hx-redirect"),
            HeaderValue::from_static("/login"),
        )],
    )
        .into_response()
}

impl IntoResponse for ActionError {
    fn into_response(self) -> Response {
        let (status, ..) = self.0.parts();
        if status == StatusCode::UNAUTHORIZED {
            return unauthorized_redirect();
        }
        Fragments::new()
            .status(status)
            .header("hx-reswap", "none")
            .toast(ToastKind::Error, &self.0.to_string())
            .map_or_else(IntoResponse::into_response, IntoResponse::into_response)
    }
}

// ------------------------------------------------------------------ extractor

/// A signed-in user, resolved from the session cookie.
///
/// * Not signed in: full-page requests are redirected to `/login`; htmx
///   requests get `401` + `HX-Redirect` so the browser navigates.
/// * State-changing requests must come from htmx (`HX-Request` header). A
///   cross-site HTML form cannot set that header, which complements the
///   `SameSite=Lax` cookie as CSRF protection.
#[derive(Debug)]
pub(crate) struct WebUser(pub(crate) AuthUser);

#[derive(Debug)]
pub(crate) enum WebRejection {
    Login { htmx: bool },
    Csrf,
    Failed { error: AppError, htmx: bool },
}

impl IntoResponse for WebRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Login { htmx: true } => unauthorized_redirect(),
            Self::Login { htmx: false } => Redirect::to("/login").into_response(),
            Self::Csrf => PageError(AppError::Forbidden(ForbiddenReason::Csrf)).into_response(),
            Self::Failed { error, htmx: true } => ActionError(error).into_response(),
            Self::Failed { error, htmx: false } => PageError(error).into_response(),
        }
    }
}

impl FromRequestParts<AppState> for WebUser {
    type Rejection = WebRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let htmx = parts.headers.contains_key(HX_REQUEST);
        let safe_method = matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS);
        if !safe_method && !htmx {
            return Err(WebRejection::Csrf);
        }
        match AuthUser::from_request_parts(parts, state).await {
            Ok(user) => Ok(Self(user)),
            Err(AppError::Unauthorized) => Err(WebRejection::Login { htmx }),
            Err(error) => Err(WebRejection::Failed { error, htmx }),
        }
    }
}
