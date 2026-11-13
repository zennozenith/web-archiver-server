//! Parsers for the multipart forms used by the archive API and web routes.

use axum::extract::Multipart;

use crate::{
    db::{collection::CollectionId, page::CreatePage},
    error::{AppError, Result},
    infra::{Image, ImageFormat, TagName, comma_separated_tags},
    service::archive::ArchiveUpload,
    util::{NonEmptyString, parse_bool, url_canon::canonicalize_url},
};

/// Fields of `POST /api/page` and the "new link" form.
pub(crate) async fn parse_page_upload(mut mp: Multipart) -> Result<CreatePage> {
    let mut upload_url: Option<String> = None;
    let mut upload_display_title: Option<NonEmptyString> = None;
    let mut upload_description: Option<String> = None;
    let mut upload_collection_id: Option<String> = None;
    let mut upload_pinned = false;
    let mut upload_tags: Vec<TagName> = Vec::new();
    let mut thumbnail_bytes: Option<Vec<u8>> = None;

    while let Some(field) = mp.next_field().await? {
        let name = field.name().unwrap_or("").to_owned();
        match name.as_str() {
            "url" => upload_url = NonEmptyString::non_empty(&field.text().await?),
            "displayTitle" => {
                upload_display_title = Some(NonEmptyString::trimmed(&field.text().await?)?);
            }
            "description" => upload_description = NonEmptyString::non_empty(&field.text().await?),
            "collectionId" => upload_collection_id = Some(field.text().await?),
            "pinned" => upload_pinned = parse_bool(&field.text().await?)?,
            "tags" => {
                let text = field.text().await?;
                upload_tags = comma_separated_tags(Some(&text))?;
            }
            "thumbnail" => {
                let bytes = field.bytes().await?.to_vec();
                // Browsers submit an empty part when no file was chosen.
                thumbnail_bytes = (!bytes.is_empty()).then_some(bytes);
            }
            _ => {}
        }
    }

    let thumbnail = match thumbnail_bytes {
        Some(bytes) => {
            let format = ImageFormat::detect(&bytes).ok_or_else(|| {
                AppError::invalid("invalid_image_format", "cannot determine thumbnail format")
            })?;
            Some(Image::new(format, bytes)?)
        }
        None => None,
    };

    let raw_url =
        upload_url.ok_or_else(|| AppError::invalid("missing_url", "A page needs a URL."))?;
    let display_title = upload_display_title.ok_or_else(|| {
        AppError::invalid("missing_display_title", "A page needs a display title.")
    })?;

    Ok(CreatePage {
        url: canonicalize_url(&raw_url)?,
        display_title,
        description: upload_description.unwrap_or_default(),
        collection_id: CollectionId::parse_or(upload_collection_id.as_ref()),
        pinned: upload_pinned,
        tags: upload_tags,
        thumbnail,
    })
}

/// Fields of `POST /api/page/{id}/capture` (and the web capture form).
pub(crate) async fn parse_capture_upload(mut mp: Multipart) -> Result<ArchiveUpload> {
    let mut upload = ArchiveUpload::default();
    let mut screenshot: Option<Vec<u8>> = None;

    while let Some(field) = mp.next_field().await? {
        let name = field.name().unwrap_or("").to_owned();
        match name.as_str() {
            "html" => upload.html = field.bytes().await?.to_vec(),
            "title" => upload.title = Some(NonEmptyString::trimmed(&field.text().await?)?),
            "screenshot" => screenshot = Some(field.bytes().await?.to_vec()),
            "pendingId" => {
                upload.pending_id = Some(
                    field
                        .text()
                        .await?
                        .trim()
                        .parse::<i64>()
                        .map_err(|_| AppError::BadRequest("pending_id must be an integer".into()))?
                        .into(),
                );
            }
            "metadata" => {
                let text = field.text().await?;
                upload.metadata = Some(serde_json::from_str(&text).map_err(|e| {
                    AppError::invalid(
                        "invalid_metadata",
                        format!("metadata is not valid JSON: {e}"),
                    )
                })?);
            }
            "saveAssets" => upload.save_assets = parse_bool(&field.text().await?)?,
            _ => {}
        }
    }

    if upload.html.is_empty() {
        return Err(AppError::invalid(
            "missing_html",
            "the html field is required and must not be empty",
        ));
    }

    if let Some(bytes) = screenshot.filter(|bytes| !bytes.is_empty()) {
        let format = ImageFormat::detect(&bytes).ok_or_else(|| {
            AppError::invalid("invalid_image_format", "cannot determine screenshot format")
        })?;
        upload.screenshot = Some(Image::new(format, bytes)?);
    }

    Ok(upload)
}
