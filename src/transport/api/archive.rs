//! Capture upload endpoint: store an archived document, its screenshot and assets.

use axum::{
    extract::{Multipart, State, multipart::MultipartRejection},
    http::StatusCode,
};

use crate::{
    app::AppState,
    db::page::PageId,
    error::Result,
    identity::AuthUser,
    service,
    transport::{
        extract::body::{AppJson, AppPath},
        multipart::parse_capture_upload,
    },
};

/// POST /page/{id}/capture
pub(super) async fn archive(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, AppJson<service::archive::ArchiveResult>)> {
    let upload = parse_capture_upload(multipart?).await?;
    let result = service::archive::archive(&state, &user, id, upload).await?;
    Ok((StatusCode::CREATED, AppJson(result)))
}
