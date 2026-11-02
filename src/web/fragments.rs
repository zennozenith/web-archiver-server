//! HTMX fragment handlers. These return partial templates, never a full page.

use axum::extract::{Path, Query, State};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    db::{
        link::{HighlightRecord, LinkField, LinkFilter, LinkItem, LinkSort},
        page::PageId,
        tag::TagSummary,
    },
    error::{OptionExt, Result},
    limit::Limit,
    template::RenderHtml,
    web::{
        templates::{FormatsTemplate, LinkContentTemplate, LinkGridTemplate, TagListTemplate},
        view::{FormatView, LinkView, TagView},
    },
};

#[derive(Debug, Deserialize)]
pub(crate) struct SearchQuery {
    q: Option<String>,
    sort: Option<String>,
    #[serde(rename = "in")]
    field: Option<String>,
}

fn parse_sort(value: Option<&str>) -> LinkSort {
    match value {
        Some("oldest") => LinkSort::Oldest,
        Some("name") => LinkSort::Name,
        _ => LinkSort::Newest,
    }
}

fn parse_field(value: Option<&str>) -> LinkField {
    match value {
        Some("url") => LinkField::Url,
        Some("text") => LinkField::Text,
        Some("tags") => LinkField::Tags,
        _ => LinkField::Name,
    }
}

pub(crate) async fn search_links(
    State(pool): State<SqlitePool>,
    Query(query): Query<SearchQuery>,
) -> Result<impl axum::response::IntoResponse> {
    let (items, _) = LinkItem::list(
        &pool,
        &LinkFilter {
            text: query.q,
            sort: parse_sort(query.sort.as_deref()),
            field: parse_field(query.field.as_deref()),
            limit: Limit::MAX,
            ..LinkFilter::default()
        },
    )
    .await?;

    let links = items
        .iter()
        .map(|item| LinkView::from_item(item, &[]))
        .collect();
    LinkGridTemplate { links }.render_html()
}

#[derive(Debug, Deserialize)]
pub(crate) struct TagSearchQuery {
    q: Option<String>,
}

pub(crate) async fn search_tags(
    State(pool): State<SqlitePool>,
    Query(query): Query<TagSearchQuery>,
) -> Result<impl axum::response::IntoResponse> {
    let needle = query.q.unwrap_or_default().to_lowercase();
    let tags = TagSummary::list(&pool)
        .await?
        .iter()
        .filter(|tag| {
            needle.is_empty()
                || tag.name.as_str().contains(&needle)
                || tag.name.to_string().contains(&needle)
        })
        .map(TagView::from_summary)
        .collect();

    TagListTemplate { tags }.render_html()
}

#[derive(Debug, Deserialize)]
pub(crate) struct FormatQuery {
    format: Option<String>,
}

pub(crate) async fn link_format(
    State(pool): State<SqlitePool>,
    Path(id): Path<PageId>,
    Query(query): Query<FormatQuery>,
) -> Result<impl axum::response::IntoResponse> {
    let key = query.format.unwrap_or_else(|| "readable".to_owned());
    let item = LinkItem::load(&pool, id).await?.or_not_found("link")?;
    let highlights = HighlightRecord::list(&pool, id).await?;
    let link = LinkView::from_item(&item, &highlights);

    let format = link
        .formats
        .iter()
        .find(|format| format.key == key)
        .cloned()
        .unwrap_or_else(|| FormatView {
            key: key.clone(),
            label: key.clone(),
            icon: "📖".to_owned(),
        });
    let readable = format.key == "readable";

    LinkContentTemplate {
        link,
        format,
        readable,
    }
    .render_html()
}

pub(crate) async fn refetch_formats(
    State(pool): State<SqlitePool>,
    Path(id): Path<PageId>,
) -> Result<impl axum::response::IntoResponse> {
    let item = LinkItem::load(&pool, id).await?.or_not_found("link")?;
    let link = LinkView::from_item(&item, &[]);
    FormatsTemplate { link }.render_html()
}
