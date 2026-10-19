use std::{
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use tokio::fs;
use url::Url;

const MIN_SIZE: usize = 1024; // below this, overhead usually wins
const KEEP_IF_BELOW: f64 = 0.90; // keep compressed only if < 90% of original
const PROBE_SIZE: usize = 64 * 1024; // sample size for large files
const PROBE_BAIL_ABOVE: f64 = 0.95; // sample barely shrinks -> skip full compression

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Define your list of URLs
    let urls = [
        "http://zenitharch:9001/static.files/normalize-9960930a.css",
        "http://zenitharch:9001/static.files/rustdoc-203241e4.css",
        "http://zenitharch:9001/static.files/favicon-32x32-eab170b8.png",
        "http://zenitharch:9001/static.files/favicon-044be391.svg",
        "http://zenitharch:9001/static.files/storage-41dd4d93.js",
        "http://zenitharch:9001/crates.js",
        "http://zenitharch:9001/static.files/main-1814ead3.js",
    ]
    .into_iter()
    .map(|v| Url::parse(v).unwrap())
    .collect::<Vec<_>>();

    // 2. Create the tmp directory if it doesn't exist
    let tmp_dir = Path::new("temp/downloaded");
    fs::create_dir_all(tmp_dir).await?;

    // 3. Initialize the HTTP client (using rustls)
    let raw_headers = vec![
        (
            "User-Agent",
            "Mozilla/5.0 (X11; Linux x86_64; rv:157.0) Gecko/20100101 Firefox/157.0",
        ),
        ("Accept", "*/*"),
        ("Accept-Language", "en-US,en;q=0.9"),
        ("Accept-Encoding", "gzip, deflate, br, zstd"),
        ("Sec-GPC", "1"),
        ("Connection", "keep-alive"),
        (
            "Referer",
            "https://docs.rs/axum/latest/axum/struct.Json.html",
        ),
        ("Sec-Fetch-Dest", "script"),
        ("Sec-Fetch-Mode", "no-cors"),
        ("Sec-Fetch-Site", "same-origin"),
        ("Priority", "u=2"),
        ("Pragma", "no-cache"),
        ("Cache-Control", "no-cache"),
        ("TE", "trailers"),
    ];

    let mut headers = HeaderMap::new();

    for (key, val) in raw_headers {
        let name = HeaderName::from_str(key)?;
        let value = HeaderValue::from_str(val)?;
        headers.insert(name, value);
    }
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;

    // 4. Set your desired concurrency limit (e.g., 5 downloads at a time)
    let concurrency_limit = 5;

    let stream = futures::stream::iter(urls.into_iter())
        .map(|url| {
            let client = client.clone();
            let tmp_dir = tmp_dir.to_path_buf();

            tokio::spawn(async move { download_and_save(&client, url, &tmp_dir).await })
        })
        .buffer_unordered(concurrency_limit);

    // 5. Execute and await all downloads
    let results = stream.collect::<Vec<_>>().await;

    // 6. Handle results/errors
    for res in results {
        match res {
            Ok(Ok(DownloadedFile {
                url,
                file_path,
                mime_type,
                hash,
                compressed,
            })) => {
                println!(
                    r#"{{
    compressed: {}, 
    file_path: {}, 
    hash: {},
    mime_type: {},
    url: {}
}}"#,
                    compressed,
                    file_path.display(),
                    hash.to_hex(),
                    mime_type,
                    url,
                );
            }
            Ok(Err(e)) => eprintln!("Download/IO error: {}", e),
            Err(e) => eprintln!("Task panicked or failed: {}", e),
        }
    }

    Ok(())
}

struct DownloadedFile {
    url: Url,
    file_path: PathBuf,
    mime_type: String,
    hash: HashType,
    compressed: bool,
}

enum HashType {
    Blake3([u8; 32]),
    #[allow(dead_code)]
    Sha256([u8; 32]),
}

impl HashType {
    pub fn to_hex(&self) -> String {
        let bytes = match self {
            HashType::Blake3(b) => b,
            HashType::Sha256(b) => b,
        };
        let mut s = String::with_capacity(64); // 32 bytes * 2 chars per byte = 64
        for b in bytes {
            use std::fmt::Write;
            let _ = write!(s, "{:02x}", b);
        }
        s
    }
}

fn extension_for(essence: &str, url: &Url) -> String {
    //  Explicit overrides for types where the crate's first pick is poor
    let preferred = match essence {
        "text/javascript" | "application/javascript" | "application/x-javascript" => Some("js"),
        "image/jpeg" => Some("jpg"),
        "text/html" => Some("html"),
        "text/plain" => Some("txt"),
        "application/json" => Some("json"),
        "image/svg+xml" => Some("svg"),
        "text/css" => Some("css"),
        _ => None,
    };
    if let Some(ext) = preferred {
        return ext.to_string();
    }

    // If the URL's extension is one of the valid extensions for this MIME type, use it
    let url_ext = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .and_then(|name| Path::new(name).extension())
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    let candidates = mime_guess::get_mime_extensions_str(essence);

    if let (Some(ue), Some(exts)) = (&url_ext, candidates)
        && exts.contains(&ue.as_str())
    {
        return ue.clone();
    }

    // Otherwise the crate's first pick, then the URL extension, then bin
    candidates
        .and_then(|exts| exts.first().copied())
        .map(str::to_string)
        .or(url_ext)
        .unwrap_or_else(|| "bin".to_string())
}

/// Cheap pre-filter: types that are already compressed, so don't even try.
fn is_already_compressed(essence: &str) -> bool {
    if essence == "image/svg+xml"
        || essence == "image/bmp"
        || essence == "image/x-icon"
        || essence == "image/vnd.microsoft.icon"
    {
        return false;
    }
    essence.starts_with("image/")
        || essence.starts_with("video/")
        || essence.starts_with("audio/")
        || matches!(
            essence,
            "application/zip"
                | "application/gzip"
                | "application/x-gzip"
                | "application/zstd"
                | "application/x-bzip2"
                | "application/x-xz"
                | "application/x-7z-compressed"
                | "application/vnd.rar"
                | "application/pdf"
                | "font/woff"
                | "font/woff2"
        )
        || essence.starts_with("application/vnd.openxmlformats-officedocument")
}

/// - quality: 0 to 11 (11 is max compression, slower; 3-4 is a good fast balance)
fn brotli_bytes(data: &[u8], quality: u32) -> std::io::Result<Vec<u8>> {
    let lgwin = 22; // window size
    let mut w =
        brotli::CompressorWriter::new(Vec::with_capacity(data.len() / 2), 4096, quality, lgwin);
    w.write_all(data)?;
    Ok(w.into_inner())
}

/// Returns (final_path, was_compressed). Runs on a blocking thread.
fn save_smart(
    file_path: &Path,
    data: &[u8],
    essence: &str,
) -> Result<(PathBuf, bool), Box<dyn std::error::Error + Send + Sync>> {
    let try_compress = data.len() >= MIN_SIZE && !is_already_compressed(essence);

    if try_compress {
        // For big inputs, probe a sample first so we don't burn CPU on incompressible data.
        let worth_trying = if data.len() > PROBE_SIZE * 4 {
            let sample = brotli_bytes(&data[..PROBE_SIZE], 1)?;
            (sample.len() as f64 / PROBE_SIZE as f64) < PROBE_BAIL_ABOVE
        } else {
            true
        };

        if worth_trying {
            let compressed = brotli_bytes(data, 6)?;
            if (compressed.len() as f64) < data.len() as f64 * KEEP_IF_BELOW {
                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("bin");
                let out_path = file_path.with_extension(format!("{ext}.br"));
                std::fs::write(&out_path, &compressed)?;
                return Ok((out_path, true));
            }
        }
    }

    std::fs::write(file_path, data)?;
    Ok((file_path.to_path_buf(), false))
}

async fn save_file_smart(
    file_path: &Path,
    bytes: bytes::Bytes,
    essence: &str,
) -> Result<(PathBuf, bool), Box<dyn std::error::Error + Send + Sync>> {
    let path = file_path.to_path_buf();
    let essence = essence.to_ascii_lowercase();
    tokio::task::spawn_blocking(move || save_smart(&path, &bytes, &essence)).await?
}

async fn download_and_save(
    client: &reqwest::Client,
    url: Url,
    tmp_dir: &Path,
) -> Result<DownloadedFile, Box<dyn std::error::Error + Send + Sync>> {
    let response = client.get(url.as_str()).send().await?;

    if !response.status().is_success() {
        return Err(format!("HTTP error status: {}", response.status()).into());
    }

    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();

    // Drop parameters such as "; charset=utf-8"
    let essence = mime_type.split(';').next().unwrap_or("").trim();

    println!("URL: {} | MIME Type: {}", url, mime_type);

    let bytes = response.bytes().await?;
    let hash = HashType::Blake3(blake3::hash(&bytes).as_bytes().to_owned());

    let file_name = hash.to_hex();
    let file_extension = extension_for(essence, &url);
    let file_path = tmp_dir.join(format!("{}.{}", file_name, file_extension));

    let (final_path, compressed) = save_file_smart(&file_path, bytes, essence).await?;

    Ok(DownloadedFile {
        url,
        file_path: final_path,
        mime_type,
        hash,
        compressed,
    })
}
