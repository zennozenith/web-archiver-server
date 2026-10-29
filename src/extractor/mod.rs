use base64::{Engine as _, engine::general_purpose::STANDARD};

pub mod css;
pub mod html;

pub fn data_uri_to_bytes(uri: &str) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
    // Format: data:[<mime>][;base64],<data>
    let rest = uri.strip_prefix("data:").ok_or("not a data URI")?;
    let (meta, data) = rest.split_once(',').ok_or("missing comma")?;

    if !meta.ends_with(";base64") {
        return Err("only base64 data URIs are supported".into());
    }
    let mime = meta.trim_end_matches(";base64").to_string();

    let bytes = STANDARD.decode(data.trim())?;
    Ok((mime, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_uri_to_bytes() {
        let uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

        let (mime, _bytes) = data_uri_to_bytes(uri).unwrap();

        let _ext = match mime.as_str() {
            "image/png" => "png",
            "image/jpeg" | "image/jpg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/svg+xml" => "svg",
            _ => "bin",
        };

        // fs::write(format!("output.{ext}"), &bytes)?;
        // println!("Saved output.{ext} ({} bytes)", bytes.len());
    }
}
