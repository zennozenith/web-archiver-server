//! Capture upload endpoint: store an archived document, its screenshot and assets.

use axum::{
    extract::{Multipart, State, multipart::MultipartRejection},
    http::StatusCode,
};

use crate::{
    app::AppState,
    auth::AuthUser,
    db::page::PageId,
    error::Result,
    http::{
        extract::body::{AppJson, AppPath},
        multipart::parse_capture_upload,
    },
    service,
};

/// POST /page/{id}/capture
pub(super) async fn archive(
    State(ctx): State<AppState>,
    user: AuthUser,
    AppPath(id): AppPath<PageId>,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, AppJson<service::archive::ArchiveResult>)> {
    let upload = parse_capture_upload(multipart?).await?;
    let result = service::archive::archive(&ctx, &user, id, upload).await?;
    Ok((StatusCode::CREATED, AppJson(result)))
}
