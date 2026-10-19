use std::{fs::File, io::Write as _, str::FromStr};

use askama::Template;
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Multipart, State,
        multipart::{MultipartError, MultipartRejection},
        rejection::JsonRejection,
    },
    response::IntoResponse,
    routing::{get, post},
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::{
    AppState,
    core::{IntoAnyhow, RenderHtml as _},
    error::AppError,
};
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    title: &'a str,
}

#[derive(Template)]
#[template(path = "vite-counter.html")]
struct ViteCounter<'a> {
    title: &'a str,
}

pub async fn index() -> Result<impl IntoResponse, AppError> {
    IndexTemplate {
        title: "Rust + Axum + Vite + Alpine",
    }
    .render_html()
}

pub async fn vite_counter() -> Result<impl IntoResponse, AppError> {
    ViteCounter {
        title: "Rust + Axum + Vite + Alpine",
    }
    .render_html()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PageSnapshot {
    ///eg: 2026-10-15T14:28:53.664Z    pub captured_at: String,
    pub cookies: Option<Vec<Cookie>>,
    pub cookies_error: Option<String>,
    pub html: String,
    pub page: PageInfo,
    pub request: Option<RequestInfo>,
    pub response: Option<ResponseInfo>,
    pub screenshot: Option<String>,
    pub tab: TabInfo,
    pub tags: Vec<String>,
    pub title: String,
    pub url: String,
    pub user_agent: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub domain: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SameSite {
    NoRestriction,
    Lax,
    Strict,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub canonical: Option<String>,
    pub charset: String,
    pub content_type: String,
    pub description: Option<String>,
    pub lang: String,
    /// eg: "09/04/2026 17:47:04",
    pub last_modified: String,
    pub ready_state: String,
    pub title: String,
    pub url: String,
    pub viewport: Viewport,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub doc_height: u64,
    pub dpr: u32,
    pub height: u32,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub width: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RequestInfo {
    pub headers: Vec<Header>,
    pub matches_page_url: bool,
    pub method: String,
    /// timestamp: 1792074510697,
    pub timestamp: u64,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResponseInfo {
    pub from_cache: bool,
    pub headers: Vec<Header>,
    pub ip: String,
    pub status: u16,
    pub status_line: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TabInfo {
    pub fav_icon_url: String,
    pub incognito: bool,
    pub status: String,
}

struct Part {
    name: Option<String>,
    filename: Option<String>,
    data: Bytes,
}

/// Accept both field names (html) and file names (page.html).
fn pick<'a>(parts: &'a [Part], names: &[&str]) -> Option<&'a Part> {
    for n in names {
        if let Some(p) = parts.iter().find(|p| p.name.as_deref() == Some(*n)) {
            return Some(p);
        }
    }
    parts
        .iter()
        .find(|p| p.filename.as_deref().is_some_and(|f| names.contains(&f)))
}

fn part_text(p: Option<&Part>) -> String {
    p.map(|p| String::from_utf8_lossy(&p.data).into_owned())
        .unwrap_or_default()
}

async fn archive(
    State(_db): State<SqlitePool>,
    mp: Result<Multipart, MultipartRejection>,
) -> Result<impl IntoResponse, AppError> {
    let mut mp = mp.map_err(|_| AppError::from("Invalid body. Expected multipart/form-data."))?;

    let mut parts = Vec::new();

    let body_err = |e: MultipartError| {
        AppError::StringError("Invalid body. Expected multipart/form-data.".into())
    };

    while let Some(field) = mp.next_field().await.map_err(body_err)? {
        let name = field.name().map(str::to_owned);
        let filename = field.file_name().map(str::to_owned);
        let data = field.bytes().await.map_err(body_err)?;
        parts.push(Part {
            name,
            filename,
            data,
        });
    }

    let html = pick(&parts, &["html", "page.html"]).map(|p| p.data.clone());
    // TODO: screenshot ext png | jpg
    // let screenshot = pick(&parts, &["screenshot", "screenshot.png"]).map(|p| p.data.clone());
    let screenshot = pick(&parts, &["screenshot", "screenshot.jpg"]).map(|p| p.data.clone());
    let meta: Option<Value> = {
        let t = part_text(pick(&parts, &["metadata", "json", "metadata.json"]));
        serde_json::from_str(&t)
            .ok()
            .filter(|v: &Value| !v.is_null())
    };
    let meta_str = |k: &str| {
        meta.as_ref()
            .and_then(|m| m.get(k))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };

    let form_url = part_text(pick(&parts, &["url"]));
    let raw_url = if form_url.is_empty() {
        meta_str("url").unwrap_or_default()
    } else {
        form_url
    };
    let url = normalize_url(&raw_url)
        .ok_or_else(|| AppError::bad("Missing or invalid target URL in archive request"))?;

    let form_title = part_text(pick(&parts, &["title"]));
    let title = if !form_title.is_empty() {
        form_title
    } else {
        meta_str("title").unwrap_or_else(|| url.clone())
    };

    let now = Utc::now();
    let archived_at = iso(now);
    let host: String = url::Url::parse(&url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let folder = format!("{}_{}", archived_at.replace([':', '.'], "-"), host);
    let dir = st.archives_dir.join(&folder);
    tokio::fs::create_dir_all(&dir).await?;

    if let Some(h) = &html {
        tokio::fs::write(dir.join("page.html"), h).await?;
    }
    if let Some(s) = &screenshot {
        tokio::fs::write(dir.join("screenshot.png"), s).await?;
    }
    if let Some(m) = &meta {
        tokio::fs::write(
            dir.join("metadata.json"),
            serde_json::to_vec_pretty(m).unwrap(),
        )
        .await?;
    }

    let id = uuid::Uuid::new_v4().to_string();
    let mut tx = st.db.begin().await?;
    sqlx::query(
        "INSERT INTO archives (id, url, title, folder, archived_at, html_size, has_screenshot, metadata)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&url)
    .bind(&title)
    .bind(&folder)
    .bind(&archived_at)
    .bind(html.as_ref().map_or(0, |h| h.len() as i64))
    .bind(screenshot.is_some() as i64)
    .bind(meta.as_ref().map(|m| m.to_string()))
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM pending WHERE url = ?")
        .bind(&url)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let version_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archives WHERE url = ?")
        .bind(&url)
        .fetch_one(&st.db)
        .await?;
    info!("[Archived] \"{title}\" -> {folder} ({version_count} versions)");

    Ok(Json(json!({
        "success": true,
        "message": "Page archived successfully",
        "id": id,
        "url": url,
        "archivedAt": archived_at,
        "versionCount": version_count,
        "folder": folder,
    })))
    // let mut file = File::create("temp/output.json").anyhow()?;

    // let json_data = serde_json::to_string_pretty(&req.0).unwrap();
    // file.write_all(json_data.as_bytes()).anyhow()?;

    // let mut file = File::create("temp/html.html").anyhow()?;
    // file.write_all(req.html.as_bytes()).anyhow()?;

    // Ok(Json(serde_json::json!({})))
}

#[derive(Debug, Serialize)]
struct PendingUrl {
    id: String,
    url: String,
    title: String,
}

async fn get_pending() -> Result<Json<Vec<PendingUrl>>, AppError> {
    Ok(Json(vec![
        PendingUrl {
            id: "1".into(),
            url: "https://a.example.com".into(),
            title: "A example".into(),
        },
        PendingUrl {
            id: "2".into(),
            url: "https://b.example.com".into(),
            title: "B example".into(),
        },
        PendingUrl {
            id: "3".into(),
            url: "https://c.example.com".into(),
            title: "C example".into(),
        },
        PendingUrl {
            id: "4".into(),
            url: "https://docs.rs/axum/latest/axum/struct.Json.html".into(),
            title: "Axum Json".into(),
        },
    ]))
}

pub fn api() -> Router<AppState> {
    Router::new()
        .route("/archive", post(archive))
        .route("/pending", get(get_pending))
        .route("/check", get(get_pending))
        .layer(DefaultBodyLimit::max(1024 * 4))
}
