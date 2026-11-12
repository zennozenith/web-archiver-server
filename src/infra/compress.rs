use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

const MIN_COMPRESS_SIZE: usize = 256;
/// Guards against decompression bombs when serving.
const MAX_DECOMPRESSED: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[repr(i64)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Compression {
    None = 0,
    Gzip = 1,
    Brotli = 2,
    // Zstd = 3,
}

impl Compression {
    pub(crate) const fn content_encoding(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Gzip => Some("gzip"),
            Self::Brotli => Some("br"),
        }
    }
}

pub(crate) fn is_compressible(mime: &str) -> bool {
    let m = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    m.starts_with("text/")
        || m.ends_with("+json")
        || m.ends_with("+xml")
        || matches!(
            m.as_str(),
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-javascript"
                | "application/xhtml+xml"
        )
}

fn io_err(e: std::io::Error) -> AppError {
    AppError::Internal(Box::new(e))
}

pub(crate) fn compress(data: &[u8], alg: Compression) -> Result<Vec<u8>> {
    match alg {
        Compression::None => Ok(data.to_vec()),
        Compression::Gzip => {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
            enc.write_all(data).map_err(io_err)?;
            enc.finish().map_err(io_err)
        }
        Compression::Brotli => {
            let mut out = Vec::with_capacity(data.len() / 3);
            {
                // quality 5 keeps ingest fast; 11 is much slower for little gain
                let mut w = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
                w.write_all(data).map_err(io_err)?;
                w.flush().map_err(io_err)?;
            } // dropped here -> stream finalized
            Ok(out)
        }
    }
}

pub(crate) fn decompress(data: &[u8], alg: Compression) -> Result<Vec<u8>> {
    let reader: Box<dyn Read + '_> = match alg {
        Compression::None => return Ok(data.to_vec()),
        Compression::Gzip => Box::new(flate2::read::GzDecoder::new(data)),
        Compression::Brotli => Box::new(brotli::Decompressor::new(data, 4096)),
    };
    let mut out = Vec::new();
    reader
        .take(MAX_DECOMPRESSED)
        .read_to_end(&mut out)
        .map_err(io_err)?;
    Ok(out)
}

/// Policy: brotli for text-like types, otherwise store raw. Falls back to raw
/// when compression doesn't save at least ~5%.
pub(crate) fn compress_for_storage(mime: &str, data: Vec<u8>) -> Result<(Vec<u8>, Compression)> {
    if data.len() < MIN_COMPRESS_SIZE || !is_compressible(mime) {
        return Ok((data, Compression::None));
    }
    let out = compress(&data, Compression::Brotli)?;
    let threshold = data.len().saturating_sub(data.len().saturating_div(20));
    if out.len() < threshold {
        Ok((out, Compression::Brotli))
    } else {
        Ok((data, Compression::None))
    }
}
