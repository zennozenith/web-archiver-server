use serde::Deserialize;

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    #[serde(alias = "jpg")]
    Jpeg,
}

impl ImageFormat {
    const ALL: [Self; 2] = [Self::Png, Self::Jpeg];

    pub const fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }

    /// Leading magic bytes that identify this format.
    const fn signature(self) -> &'static [u8] {
        match self {
            Self::Png => &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
            Self::Jpeg => &[0xFF, 0xD8, 0xFF],
        }
    }

    /// Whether `data` begins with this format's magic bytes.
    pub fn matches_signature(self, data: &[u8]) -> bool {
        data.starts_with(self.signature())
    }

    /// Sniff the format from magic bytes, if it is one we support.
    pub fn detect(data: &[u8]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|format| format.matches_signature(data))
    }

    pub fn parse_from_extension(s: &str) -> Result<Self, AppError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "png" => Ok(Self::Png),
            "jpeg" | "jpg" => Ok(Self::Jpeg),
            _ => Err(AppError::invalid(
                "invalid_screenshot_format",
                "screenshot-format must be \"png\" or \"jpeg\"",
            )),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("data does not look like a {claimed:?} image (magic bytes mismatch)")]
pub struct FormatMismatch {
    pub claimed: ImageFormat,
}

impl From<FormatMismatch> for AppError {
    fn from(err: FormatMismatch) -> Self {
        Self::invalid("invalid_image_format", err.to_string())
    }
}

/// An image whose bytes are known to match its declared format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Image {
    format: ImageFormat,
    data: Vec<u8>,
}

impl Image {
    pub(crate) fn new(format: ImageFormat, data: Vec<u8>) -> Result<Self, FormatMismatch> {
        if format.matches_signature(&data) {
            Ok(Self { format, data })
        } else {
            Err(FormatMismatch { claimed: format })
        }
    }

    pub(crate) const fn format(&self) -> ImageFormat {
        self.format
    }

    #[expect(dead_code, reason = "Might be needed later")]
    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn into_data(self) -> Vec<u8> {
        self.data
    }
}
