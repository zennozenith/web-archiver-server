use std::collections::HashMap;

use crate::{
    db::{
        self,
        pages::PageRow,
        stats::{self, Stats},
        tags::TagRow,
    },
    dto,
    error::Result,
    template::RenderHtml,
    time::ago_timestamp,
};

use super::{
    pages::{CaptureView, PageCardTemplate, PageTag},
    pending::PendingFragmentTemplate,
    tags::TagView,
};

use askama::Template;
use axum::{extract::State, response::IntoResponse};
use sqlx::SqlitePool;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    pendings: Vec<PendingFragmentTemplate<'a>>,
    stats: Stats,
    pages: Vec<PageCardTemplate>,
    tags: Vec<TagView>,
    app_script: String,
}

#[derive(Template)]
#[template(path = "vite-counter.html")]
struct ViteCounter<'a> {
    title: &'a str,
}

pub(super) async fn index(State(pool): State<SqlitePool>) -> Result<impl IntoResponse> {
    const LIMIT: u32 = 50;

    let stats = stats::stats(&pool).await?;

    let dto::Page { items, .. } = db::pending::list_pending(&pool, None, None, 10).await?;
    let pendings = items
        .iter()
        .map(PendingFragmentTemplate::from)
        .collect::<Vec<_>>();

    let rows = db::pages::list_pages(&pool, LIMIT).await?;
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let mut tags_by_page = db::pages::tags_for_pages(&pool, &ids).await?;
    let mut captures_by_page = db::pages::captures_for_pages(&pool, &ids).await?;

    let pages = rows
        .into_iter()
        .map(|row| page_card(row, &mut tags_by_page, &mut captures_by_page))
        .collect();

    let tags = db::tags::list_tag_counts(&pool)
        .await?
        .into_iter()
        .map(|tag| TagView {
            id: tag.id,
            name: tag.name,
            pages: tag.pages,
            pending: tag.pending,
        })
        .collect();

    IndexTemplate {
        pendings,
        stats,
        pages,
        tags,
        app_script: crate::assets::vite_asset("src/app.ts").unwrap_or_default(),
    }
    .render_html()
}

#[expect(
    clippy::too_many_lines,
    reason = "builds the complete view model for one page card"
)]
fn page_card(
    row: PageRow,
    tags_by_page: &mut HashMap<i64, Vec<TagRow>>,
    captures_by_page: &mut HashMap<i64, Vec<db::captures::CaptureForPageRow>>,
) -> PageCardTemplate {
    let PageRow {
        id,
        url,
        host,
        last_captured_at,
        capture_count,
        title,
    } = row;

    let display_title = title
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| url.clone());

    let tags = tags_by_page
        .remove(&id)
        .unwrap_or_default()
        .iter()
        .map(PageTag::from)
        .collect::<Vec<_>>();
    let tag_names = tags
        .iter()
        .map(|tag| tag.name.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    let capture_rows = captures_by_page.remove(&id).unwrap_or_default();
    let total_versions = i64::try_from(capture_rows.len()).unwrap_or(i64::MAX);
    let captures = capture_rows
        .into_iter()
        .enumerate()
        .map(|(index, capture)| {
            let index = i64::try_from(index).unwrap_or(i64::MAX);
            let version = total_versions.saturating_sub(index);
            let capture_title = capture
                .title
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("Version {version}"));
            CaptureView {
                id: capture.id,
                version,
                title: capture_title,
                captured_at: ago_timestamp(capture.captured_at),
                status: capture
                    .status_code
                    .map_or_else(|| "—".to_owned(), |code| code.to_string()),
                has_html: capture.has_html,
                has_screenshot: capture.has_screenshot,
            }
        })
        .collect();

    PageCardTemplate {
        id,
        search: format!("{display_title} {url} {tag_names}").to_lowercase(),
        url,
        host,
        title: display_title,
        last_captured: if capture_count > 0 {
            ago_timestamp(last_captured_at)
        } else {
            "never archived".to_owned()
        },
        version_label: if capture_count == 1 {
            "1 version".to_owned()
        } else {
            format!("{capture_count} versions")
        },
        capture_count,
        tags,
        captures,
    }
}

pub(super) async fn vite_counter() -> Result<impl IntoResponse> {
    ViteCounter {
        title: "Rust + Axum + Vite + Alpine",
    }
    .render_html()
}
