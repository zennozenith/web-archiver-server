//! Security headers applied to every response, and the CSP report sink.
//!
//! Archived content sets its own, stricter CSP and `nosniff` (see `serve.rs`);
//! this layer only fills in headers the inner response did not set, and never
//! reports archived pages against the app policy.
//!
//! The strict app policy is rolled out in two stages ([`CspMode`]):
//!
//! * `report_only` (default): a transitional policy is enforced and the strict
//!   one is sent as `Content-Security-Policy-Report-Only`, so violations show up
//!   in the log (`/csp-report`) without breaking anything.
//! * `enforce`: the strict policy is enforced, and still reports.
//!
//! Report-only mode finds CSP *violations* only. It does not find Alpine
//! expressions the CSP build cannot evaluate: those are ordinary console
//! errors, so click through the UI once with the strict policy enforced
//! locally before flipping the switch.

use std::sync::Arc;

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Request, State},
    http::{
        HeaderName, HeaderValue, StatusCode,
        header::{
            CONTENT_SECURITY_POLICY, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
        },
    },
    middleware::Next,
    response::Response,
    routing::post,
};

use crate::{app::{AppState, RuntimeConfig}, config::CspMode};

const REPORT_PATH: &str = "/csp-report";
const MAX_REPORT_BYTES: usize = 16 * 1024;
const MAX_LOGGED_REPORT_CHARS: usize = 2048;

// `const fn`s rather than `const` items: `HeaderName` has interior mutability,
// which `clippy::declare_interior_mutable_const` rejects (same as `app.rs`).
const fn report_only_header() -> HeaderName {
    HeaderName::from_static("content-security-policy-report-only")
}

const fn reporting_endpoints_header() -> HeaderName {
    HeaderName::from_static("reporting-endpoints")
}

/// Directives shared by the transitional and strict policies: self-hosted
/// fonts, inline styles from the templates, no framing.
macro_rules! shared_directives {
    () => {
        "default-src 'self'; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; \
         font-src 'self' data:; \
         connect-src 'self'; \
         frame-ancestors 'none'; \
         base-uri 'self'; \
         form-action 'self'; \
         object-src 'none'"
    };
}

/// Strict policy: same-origin scripts only. Requires the Alpine CSP build
/// (`@alpinejs/csp`), which has no `eval` and no inline expressions.
const STRICT_CSP: &str = concat!(
    "script-src 'self'; ",
    shared_directives!(),
    "; report-uri /csp-report; report-to csp"
);

/// What is actually enforced while the strict policy is only being observed.
const TRANSITIONAL_CSP: &str = concat!(
    "script-src 'self' 'unsafe-inline' 'unsafe-eval'; ",
    shared_directives!()
);

pub(crate) async fn headers(
    State(config): State<Arc<RuntimeConfig>>,
    req: Request,
    next: Next,
) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    headers
        .entry(REFERRER_POLICY)
        .or_insert(HeaderValue::from_static("no-referrer"));
    headers
        .entry(X_CONTENT_TYPE_OPTIONS)
        .or_insert(HeaderValue::from_static("nosniff"));
    headers
        .entry(X_FRAME_OPTIONS)
        .or_insert(HeaderValue::from_static("DENY"));

    // Responses that already carry a CSP (archived content) keep theirs.
    if !headers.contains_key(CONTENT_SECURITY_POLICY) {
        headers.insert(
            reporting_endpoints_header(),
            HeaderValue::from_static(r#"csp="/csp-report""#),
        );
        match config.csp {
            CspMode::Enforce => {
                headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(STRICT_CSP));
            }
            CspMode::ReportOnly => {
                headers.insert(
                    CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static(TRANSITIONAL_CSP),
                );
                headers.insert(
                    report_only_header(),
                    HeaderValue::from_static(STRICT_CSP),
                );
            }
        }
    }

    response
}

/// Browsers POST violation reports here (`application/csp-report` or
/// `application/reports+json`). Unauthenticated by necessity, so the body is
/// size-limited and only logged, never parsed or stored.
pub(crate) fn report_router() -> Router<AppState> {
    Router::new().route(
        REPORT_PATH,
        post(csp_report).layer(DefaultBodyLimit::max(MAX_REPORT_BYTES)),
    )
}

async fn csp_report(body: Bytes) -> StatusCode {
    let text = String::from_utf8_lossy(&body);
    let report: String = text.chars().take(MAX_LOGGED_REPORT_CHARS).collect();
    // `?` escapes control characters, so a report cannot forge log lines.
    tracing::warn!(report = ?report, "content security policy report");
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use axum::{
        body::Body, http::Request, middleware::from_fn_with_state, routing::get,
    };
    use tower::ServiceExt as _;

    use super::*;

    fn config(csp: CspMode) -> Arc<RuntimeConfig> {
        Arc::new(RuntimeConfig {
            public_url: "http://localhost".to_owned(),
            secure_cookie: false,
            trusted_proxies: Vec::new(),
            csp,
        })
    }

    async fn respond(csp: CspMode, own_csp: Option<&'static str>) -> Response {
        let router = Router::new()
            .route(
                "/",
                get(move || async move {
                    let mut response = Response::new(Body::empty());
                    if let Some(value) = own_csp {
                        response
                            .headers_mut()
                            .insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(value));
                    }
                    response
                }),
            )
            .layer(from_fn_with_state(config(csp), headers));
        router
            .oneshot(Request::builder().uri("/").body(Body::empty()).expect("request"))
            .await
            .expect("response")
    }

    /// `concat!` needs literals, so the path appears in the policy strings and
    /// the `Reporting-Endpoints` value as well as in `REPORT_PATH`.
    #[test]
    fn report_path_matches_the_policy_literals() {
        assert!(STRICT_CSP.contains(&format!("report-uri {REPORT_PATH}")));
    }

    #[tokio::test]
    async fn report_only_enforces_transitional_and_reports_strict() {
        let response = respond(CspMode::ReportOnly, None).await;
        let enforced = response.headers()[CONTENT_SECURITY_POLICY].to_str().expect("ascii");
        let observed = response.headers()[report_only_header()]
            .to_str()
            .expect("ascii");
        assert!(enforced.contains("'unsafe-eval'"));
        assert!(!observed.contains("'unsafe-eval'"));
        assert!(observed.contains("report-uri /csp-report"));
        assert!(response.headers().contains_key(reporting_endpoints_header()));
    }

    #[tokio::test]
    async fn enforce_mode_serves_only_the_strict_policy() {
        let response = respond(CspMode::Enforce, None).await;
        let enforced = response.headers()[CONTENT_SECURITY_POLICY].to_str().expect("ascii");
        assert!(!enforced.contains("'unsafe-inline' 'unsafe-eval'"));
        assert!(enforced.contains("script-src 'self';"));
        assert!(!response.headers().contains_key(report_only_header()));
    }

    #[tokio::test]
    async fn archived_content_keeps_its_own_policy() {
        let response = respond(CspMode::ReportOnly, Some("sandbox")).await;
        assert_eq!(response.headers()[CONTENT_SECURITY_POLICY], "sandbox");
        assert!(!response.headers().contains_key(report_only_header()));
        assert_eq!(response.headers()[REFERRER_POLICY], "no-referrer");
    }

    /// The strict policy has no `unsafe-eval`, so the standard Alpine build
    /// would fail silently. Skipped when the frontend is not checked out.
    #[test]
    fn frontend_imports_only_the_csp_build_of_alpine() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend/src");
        let Ok(entry) = std::fs::read_to_string(root.join("app.ts")) else {
            return;
        };
        assert!(
            entry.contains(r#"from "@alpinejs/csp""#),
            "app.ts must import the CSP build of Alpine"
        );

        let mut files = Vec::new();
        collect_ts(&root, &mut files);
        for file in files {
            let source = std::fs::read_to_string(&file).expect("readable source");
            for line in source.lines().map(str::trim_start) {
                let value_import = line.starts_with("import ") && !line.starts_with("import type");
                assert!(
                    !(value_import && line.contains(r#"from "alpinejs""#)),
                    "{}: runtime import of the standard Alpine build: {line}",
                    file.display()
                );
            }
        }
    }

    fn collect_ts(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("readable dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_ts(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "ts") {
                out.push(path);
            }
        }
    }
}
