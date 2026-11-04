use axum::{
    Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;

use crate::{
    db::{
        collection::{CollectionId, CollectionRecord},
        helper::CursorPage,
        page::{HighlightRecord, PageCursor, PageFilter, PageId, PageItem, PageSort, PageStats},
        tag::TagSummary,
    },
    error::{AppError, OptionExt, Result},
    limit::Limit,
    tag::TagName,
    template::RenderHtml,
    web::{
        templates::{
            ArchiveTemplate, CollectionFormTemplate, CollectionTemplate, CollectionsTemplate,
            DashboardTemplate, PageDetailTemplate, PageGridTemplate, PagesTemplate, PinnedTemplate,
            PublicCollectionTemplate, TagTemplate, TagsTemplate,
        },
        view::{CollectionOption, CollectionView, FormatView, Page, PageView, Section, TagView},
    },
};

const PER_PAGE: Limit = Limit::from_const(12);
const ALL_LINKS: Limit = Limit::MAX;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageQuery {
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParentQuery {
    parent: Option<CollectionId>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditQuery {
    edit: Option<String>,
}

fn link_views(items: &[PageItem]) -> Vec<PageView> {
    items
        .iter()
        .map(|item| PageView::from_item(item, &[]))
        .collect()
}

fn collection_options(
    collections: &[CollectionRecord],
    selected: Option<CollectionId>,
) -> Vec<CollectionOption> {
    collections
        .iter()
        .map(|collection| {
            CollectionOption::from_record(collection, selected == Some(collection.id))
        })
        .collect()
}

// ───────────────────────────────────────────────────────────────────────────
// Dashboard
// ───────────────────────────────────────────────────────────────────────────

pub(crate) async fn dashboard(State(pool): State<SqlitePool>) -> Result<impl IntoResponse> {
    let stats: PageStats = crate::db::page::stats(&pool).await?;

    let CursorPage { items: recent, .. } = PageItem::list(
        &pool,
        &PageFilter {
            limit: Limit::from_const(5),
            ..PageFilter::default()
        },
    )
    .await?;

    let mut sections = vec![Section {
        title: "Recently added".to_owned(),
        href: "/links".to_owned(),
        links: link_views(&recent),
    }];

    for collection in CollectionRecord::list(&pool).await? {
        let CursorPage { items, .. } = PageItem::list(
            &pool,
            &PageFilter {
                collection_id: Some(collection.id),
                limit: Limit::from_const(5),
                ..PageFilter::default()
            },
        )
        .await?;

        sections.push(Section {
            title: format!("{} {}", collection.emoji, collection.name),
            href: format!("/collections/{}", collection.id),
            links: link_views(&items),
        });
    }

    let page = Page::load(&pool, "dashboard").await?;
    DashboardTemplate {
        page,
        stats,
        sections,
    }
    .render_html()
}

// ───────────────────────────────────────────────────────────────────────────
// Pages
// ───────────────────────────────────────────────────────────────────────────

pub(crate) async fn links(
    State(pool): State<SqlitePool>,
    Query(query): Query<PageQuery>,
) -> Result<Response> {
    let cursor = query.cursor.map(PageCursor::try_from).transpose()?;

    let CursorPage { items, next_cursor } = PageItem::list(
        &pool,
        &PageFilter {
            limit: PER_PAGE,
            cursor,
            sort: PageSort::Newest,
            ..PageFilter::default()
        },
    )
    .await?;
    let links = link_views(&items);

    // Infinite scroll asks for page 2+ and only wants the cards.
    if next_cursor.is_some() {
        return Ok(PageGridTemplate { links }.render_html()?.into_response());
    }

    let page = Page::load(&pool, "links").await?;
    Ok(PagesTemplate {
        page,
        links,
        next_cursor,
    }
    .render_html()?
    .into_response())
}

pub(crate) async fn pinned(State(pool): State<SqlitePool>) -> Result<impl IntoResponse> {
    let CursorPage { items, .. } = PageItem::list(
        &pool,
        &PageFilter {
            pinned_only: true,
            limit: ALL_LINKS,
            ..PageFilter::default()
        },
    )
    .await?;
    let page = Page::load(&pool, "pinned").await?;
    PinnedTemplate {
        page,
        links: link_views(&items),
    }
    .render_html()
}

pub(crate) async fn link_detail(
    State(pool): State<SqlitePool>,
    Path(id): Path<PageId>,
    Query(query): Query<EditQuery>,
) -> Result<impl IntoResponse> {
    let item = PageItem::load(&pool, id).await?.or_not_found("link")?;
    let highlights = HighlightRecord::list(&pool, id).await?;
    let link = PageView::from_item(&item, &highlights);
    let selected = link.collection_id;
    let collections = collection_options(&CollectionRecord::list(&pool).await?, selected);
    let page = Page::load(&pool, "links").await?;
    let edit = query.edit.is_some();

    PageDetailTemplate {
        page,
        link,
        collections,
        edit,
    }
    .render_html()
}

pub(crate) async fn archive(
    State(pool): State<SqlitePool>,
    Path((id, format)): Path<(PageId, String)>,
) -> Result<Response> {
    let item = PageItem::load(&pool, id).await?.or_not_found("link")?;

    let target = match format.as_str() {
        "readable" => Some(format!("/links/{id}")),
        "webpage" | "html" if item.record.has_html => Some(format!("/api/pages/{id}/latest/html")),
        "screenshot" | "png" if item.record.has_screenshot => {
            Some(format!("/api/pages/{id}/latest/screenshot"))
        }
        _ => None,
    };

    if let Some(target) = target {
        return Ok(Redirect::temporary(&target).into_response());
    }

    let link = PageView::from_item(&item, &[]);
    let format = link
        .formats
        .iter()
        .find(|candidate| candidate.key == format)
        .cloned()
        .unwrap_or_else(|| FormatView {
            key: format.clone(),
            label: format.clone(),
            icon: "📦".to_owned(),
        });
    let page = Page::load(&pool, "links").await?;

    Ok(ArchiveTemplate { page, link, format }
        .render_html()?
        .into_response())
}

// ───────────────────────────────────────────────────────────────────────────
// Collections
// ───────────────────────────────────────────────────────────────────────────

pub(crate) async fn collections(State(pool): State<SqlitePool>) -> Result<impl IntoResponse> {
    let collections = CollectionRecord::list(&pool)
        .await?
        .into_iter()
        .map(CollectionView::from_record)
        .collect();
    let page = Page::load(&pool, "collections").await?;

    CollectionsTemplate { page, collections }.render_html()
}

pub(crate) async fn collection(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
) -> Result<impl IntoResponse> {
    let record = CollectionRecord::load(&pool, id)
        .await?
        .or_not_found("collection")?;
    let CursorPage { items, .. } = PageItem::list(
        &pool,
        &PageFilter {
            collection_id: Some(id),
            limit: ALL_LINKS,
            ..PageFilter::default()
        },
    )
    .await?;

    let page = Page::load(&pool, "collections").await?;
    CollectionTemplate {
        page,
        collection: CollectionView::from_record(record),
        links: link_views(&items),
    }
    .render_html()
}

pub(crate) async fn collection_new(
    State(pool): State<SqlitePool>,
    Query(query): Query<ParentQuery>,
) -> Result<impl IntoResponse> {
    let collections = collection_options(&CollectionRecord::list(&pool).await?, query.parent);
    let page = Page::load(&pool, "collections").await?;

    CollectionFormTemplate {
        page,
        editing: false,
        form_title: "New Collection".to_owned(),
        submit_label: "Create collection".to_owned(),
        form_action: "/collections".to_owned(),
        name: String::new(),
        description: String::new(),
        color: "#6d5dfc".to_owned(),
        collections,
    }
    .render_html()
}

pub(crate) async fn collection_edit(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
) -> Result<impl IntoResponse> {
    let record = CollectionRecord::load(&pool, id)
        .await?
        .or_not_found("collection")?;
    let collections = collection_options(&CollectionRecord::list(&pool).await?, record.parent_id);
    let page = Page::load(&pool, "collections").await?;

    CollectionFormTemplate {
        page,
        editing: true,
        form_title: "Edit Collection".to_owned(),
        submit_label: "Save changes".to_owned(),
        form_action: format!("/collections/{id}"),
        name: record.name,
        description: record.description,
        color: record.color,
        collections,
    }
    .render_html()
}

pub(crate) async fn public_collection(
    State(pool): State<SqlitePool>,
    Path(id): Path<CollectionId>,
) -> Result<impl IntoResponse> {
    let record = CollectionRecord::load(&pool, id)
        .await?
        .or_not_found("collection")?;
    if !record.public {
        return Err(AppError::NotFound("public collection"));
    }

    let CursorPage { items, .. } = PageItem::list(
        &pool,
        &PageFilter {
            collection_id: Some(id),
            limit: ALL_LINKS,
            ..PageFilter::default()
        },
    )
    .await?;
    let page = Page::load(&pool, "collections").await?;

    PublicCollectionTemplate {
        page,
        collection: CollectionView::from_record(record),
        links: link_views(&items),
    }
    .render_html()
}

// ───────────────────────────────────────────────────────────────────────────
// Tags
// ───────────────────────────────────────────────────────────────────────────

pub(crate) async fn tags(State(pool): State<SqlitePool>) -> Result<impl IntoResponse> {
    let tags = TagSummary::list(&pool)
        .await?
        .iter()
        .map(TagView::from_summary)
        .collect();
    let page = Page::load(&pool, "tags").await?;

    TagsTemplate { page, tags }.render_html()
}

pub(crate) async fn tag(
    State(pool): State<SqlitePool>,
    Path(key): Path<String>,
) -> Result<impl IntoResponse> {
    let name: TagName = key.parse()?;
    let summary = TagSummary::find_by_name(&pool, &name)
        .await?
        .or_not_found("tag")?;
    let tag = TagView::from_summary(&summary);

    let CursorPage { items, .. } = PageItem::list(
        &pool,
        &PageFilter {
            tag_id: Some(summary.id),
            limit: ALL_LINKS,
            ..PageFilter::default()
        },
    )
    .await?;

    let page = Page::load(&pool, &tag.slug).await?;
    TagTemplate {
        page,
        tag,
        links: link_views(&items),
    }
    .render_html()
}

// ───────────────────────────────────────────────────────────────────────────
// Export
// ───────────────────────────────────────────────────────────────────────────

pub(crate) async fn export(State(pool): State<SqlitePool>) -> Result<Json<serde_json::Value>> {
    let collections = CollectionRecord::list(&pool).await?;
    let CursorPage { items, .. } = PageItem::list(
        &pool,
        &PageFilter {
            limit: ALL_LINKS,
            ..PageFilter::default()
        },
    )
    .await?;

    let links: Vec<serde_json::Value> = items
        .iter()
        .map(|item| {
            json!({
                "url": &item.record.url,
                "displayTitle": &item.record.display_title,
                "description": &item.record.description,
                "collectionId": item.record.collection_id,
                "pinned": item.record.pinned,
                "tags": item.tags.iter().map(|tag| tag.name.to_string()).collect::<Vec<_>>(),
                "createdAt": item.record.created_at,
            })
        })
        .collect();

    Ok(Json(json!({
        "collections": collections.iter().map(|collection| json!({
            "id": collection.id,
            "name": collection.name,
            "emoji": collection.emoji,
            "description": collection.description,
            "color": collection.color,
            "parentId": collection.parent_id,
            "public": collection.public,
        })).collect::<Vec<_>>(),
        "links": links,
    })))
}
