//! Raw deduplicated content blobs, addressed by their BLAKE3 hash.

use axum::{
    Router,
    extract::State,
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_LENGTH, CONTENT_TYPE, ETAG},
    },
    response::{IntoResponse, Response},
    routing::get,
};

use crate::{
    app::AppState,
    error::{AppError, Result},
    identity::AuthUser,
    infra::Blake3Hash,
    service,
    transport::{
        cache,
        extract::body::AppPath,
        serve::{etag_for, serve_content},
    },
};

async fn get_content(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    AppPath(hash): AppPath<String>,
) -> Result<Response> {
    let hash: Blake3Hash = hash.parse()?;
    let content = service::capture::content(&state.db, &user, &hash).await?;
    let content_type = content.mime_type.clone();

    serve_content(
        &state.store,
        &headers,
        content,
        &content_type,
        cache::PRIVATE_IMMUTABLE,
    )
    .await
}

async fn head_content(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(hash): AppPath<String>,
) -> Result<Response> {
    let hash: Blake3Hash = hash.parse()?;
    let head = service::capture::content_head(&state.db, &user, &hash).await?;

    let etag = HeaderValue::from_str(&etag_for(&hash)).map_err(AppError::internal)?;
    let length = u64::try_from(head.size_original).unwrap_or_default();

    let mut headers = HeaderMap::new();
    headers.insert(ETAG, etag);
    headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    if let Ok(mime) = HeaderValue::from_str(head.mime_type.as_str()) {
        headers.insert(CONTENT_TYPE, mime);
    }
    Ok((StatusCode::OK, headers).into_response())
}

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/content/{hash}", get(get_content).head(head_content))
}
