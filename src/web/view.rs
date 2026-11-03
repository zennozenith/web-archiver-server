//! Owned view models handed to Askama. Nothing here borrows from the database.

use chrono::{DateTime, Utc};
use sqlx::{Result, SqlitePool};

use crate::{
    db::{
        collection::{CollectionId, CollectionRecord},
        page::{HighlightRecord, PageItem, PageRecord},
        tag::{Tag, TagSummary},
    },
    tag::TagId,
};

const PLACEHOLDER_THUMB: &str = "/static/vite/images/hero.png";

// ───────────────────────────────────────────────────────────────────────────
// Shared page context
// ───────────────────────────────────────────────────────────────────────────

pub(crate) struct Page {
    pub csrf_token: String,
    pub nav_collections: Vec<NavCollection>,
    pub nav_tags: Vec<NavTag>,
    pub all_collections: Vec<CollectionOption>,
    pub active: String,
}

pub(crate) struct NavCollection {
    pub id: CollectionId,
    pub name: String,
    pub emoji: String,
    pub link_count: i64,
}

pub(crate) struct NavTag {
    pub name: String,
    pub slug: String,
    pub count: i64,
}

pub(crate) struct CollectionOption {
    pub id: CollectionId,
    pub name: String,
    pub emoji: String,
    pub selected: bool,
}

pub(crate) struct CollectionView {
    pub id: CollectionId,
    pub name: String,
    pub emoji: String,
    pub description: String,
    pub color: String,
    pub link_count: i64,
    pub public: bool,
    pub parent_id: Option<CollectionId>,
    pub share_url: String,
}

pub(crate) struct TagView {
    pub id: TagId,
    pub name: String,
    pub slug: String,
    pub count: i64,
}

pub(crate) struct HighlightView {
    pub text: String,
    pub color: String,
}

#[derive(Clone)]
pub(crate) struct FormatView {
    pub key: String,
    pub label: String,
    pub icon: String,
}

pub(crate) struct PageView {
    pub id: crate::db::page::PageId,
    pub title: String,
    pub url: String,
    pub domain: String,
    pub description: String,
    pub collection_id: Option<CollectionId>,
    pub collection_name: String,
    pub collection_emoji: String,
    pub collection_href: String,
    pub date: String,
    pub thumb: String,
    pub pinned: bool,
    pub tags: Vec<TagView>,
    pub tags_csv: String,
    pub formats: Vec<FormatView>,
    pub highlights: Vec<HighlightView>,
}

pub(crate) struct Section {
    pub title: String,
    pub href: String,
    pub links: Vec<PageView>,
}

// ───────────────────────────────────────────────────────────────────────────
// Builders
// ───────────────────────────────────────────────────────────────────────────

impl Page {
    pub(crate) async fn load(pool: &SqlitePool, active: &str) -> Result<Self> {
        let collections = CollectionRecord::list(pool).await?;
        let tags = TagSummary::list(pool).await?;

        Ok(Self {
            // There is no login in this single-user build, but templates keep
            // a CSRF header slot for deployments that add one later.
            csrf_token: "local".to_owned(),
            nav_collections: collections
                .iter()
                .map(|collection| NavCollection {
                    id: collection.id,
                    name: collection.name.clone(),
                    emoji: collection.emoji.clone(),
                    link_count: collection.link_count,
                })
                .collect(),
            nav_tags: tags
                .iter()
                .map(|tag| NavTag {
                    name: tag.name.to_string(),
                    slug: tag.name.to_string(),
                    count: tag.page_count,
                })
                .collect(),
            all_collections: collections
                .iter()
                .map(|collection| CollectionOption::from_record(collection, false))
                .collect(),
            active: active.to_owned(),
        })
    }
}

impl CollectionOption {
    pub(crate) fn from_record(record: &CollectionRecord, selected: bool) -> Self {
        Self {
            id: record.id,
            name: record.name.clone(),
            emoji: record.emoji.clone(),
            selected,
        }
    }
}

impl CollectionView {
    pub(crate) fn from_record(record: CollectionRecord) -> Self {
        Self {
            id: record.id,
            name: record.name,
            emoji: record.emoji,
            description: record.description,
            color: record.color,
            link_count: record.link_count,
            public: record.public,
            parent_id: record.parent_id,
            share_url: format!("/public/collections/{}", record.id),
        }
    }
}

impl TagView {
    pub(crate) fn from_summary(tag: &TagSummary) -> Self {
        Self {
            id: tag.id,
            name: tag.name.to_string(),
            slug: tag.name.to_string(),
            count: tag.page_count,
        }
    }

    fn from_tag(tag: &Tag) -> Self {
        Self {
            id: tag.id,
            name: tag.name.to_string(),
            slug: tag.name.to_string(),
            count: 0,
        }
    }
}

impl HighlightView {
    fn from_record(record: &HighlightRecord) -> Self {
        Self {
            text: record.text.clone(),
            color: record.color.clone(),
        }
    }
}

impl PageView {
    pub(crate) fn from_item(item: &PageItem, highlights: &[HighlightRecord]) -> Self {
        let record = &item.record;

        let (collection_name, collection_emoji, collection_href) =
            record.collection_id.map_or_else(
                || {
                    (
                        "Unorganized".to_owned(),
                        "📥".to_owned(),
                        "/collections".to_owned(),
                    )
                },
                |id| {
                    (
                        record
                            .collection_name
                            .clone()
                            .unwrap_or_else(|| "Collection".to_owned()),
                        record
                            .collection_emoji
                            .clone()
                            .unwrap_or_else(|| "📁".to_owned()),
                        format!("/collections/{id}"),
                    )
                },
            );

        let thumb = match record.thumbnail_content_id {
            Some(content_id) => format!("/api/page/{content_id}/thumbnail"),
            _ if record.has_screenshot => {
                format!("/api/page/{}/latest/screenshot", record.id)
            }
            _ => PLACEHOLDER_THUMB.to_owned(),
        };

        let tags: Vec<TagView> = item.tags.iter().map(TagView::from_tag).collect();
        let tags_csv = item
            .tags
            .iter()
            .map(|tag| tag.name.to_string())
            .collect::<Vec<_>>()
            .join(", ");

        Self {
            id: record.id,
            title: record.display_title.clone(),
            url: record.url.clone(),
            domain: domain_of(record),
            description: record.description.clone(),
            collection_id: record.collection_id,
            collection_name,
            collection_emoji,
            collection_href,
            date: format_date(record.created_at),
            thumb,
            pinned: record.pinned,
            tags,
            tags_csv,
            formats: formats_of(record),
            highlights: highlights.iter().map(HighlightView::from_record).collect(),
        }
    }
}

fn domain_of(record: &PageRecord) -> String {
    record
        .host
        .strip_prefix("www.")
        .unwrap_or(&record.host)
        .to_owned()
}

fn formats_of(record: &PageRecord) -> Vec<FormatView> {
    let readable = FormatView {
        key: "readable".to_owned(),
        label: "Readable".to_owned(),
        icon: "📖".to_owned(),
    };

    let mut formats = vec![readable];

    if record.has_html {
        formats.push(FormatView {
            key: "webpage".to_owned(),
            label: "Webpage".to_owned(),
            icon: "🌐".to_owned(),
        });
    }

    if record.has_screenshot {
        formats.push(FormatView {
            key: "screenshot".to_owned(),
            label: "Screenshot".to_owned(),
            icon: "🖼".to_owned(),
        });
    }

    formats
}

fn format_date(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0).map_or_else(
        || "Unknown date".to_owned(),
        |date| date.format("%b %-d, %Y").to_string(),
    )
}
