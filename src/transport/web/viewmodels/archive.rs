//! Display-ready view-models for the archive pages.

use crate::{
    db::{
        capture::CaptureSummary,
        collection::CollectionRecord,
        page::{HighlightRecord, PageItem},
        pending::{PendingResponse, PendingStatus},
        tag::TagSummary,
    },
    transport::web::viewmodels::Stamp,
};

fn thumb_url(page_id: i64) -> String {
    format!("/api/page/{page_id}/thumbnail")
}

#[derive(Debug, Clone)]
pub(crate) struct TagChip {
    pub(crate) name: String,
    pub(crate) href: String,
}

impl TagChip {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            href: format!("/tags/{name}"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CollectionBadge {
    pub(crate) emoji: String,
    pub(crate) name: String,
    pub(crate) href: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LinkCard {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) url: String,
    pub(crate) domain: String,
    pub(crate) pinned: bool,
    pub(crate) has_thumbnail: bool,
    pub(crate) thumb_url: String,
    pub(crate) collection: Option<CollectionBadge>,
    pub(crate) date: Stamp,
    pub(crate) capture_count: i64,
    pub(crate) tags: Vec<TagChip>,
    /// Lower-cased haystack used by the client-side `pageFilter` component.
    pub(crate) search: String,
}

impl LinkCard {
    pub(crate) fn new(item: PageItem) -> Self {
        let tags: Vec<TagChip> = item
            .tags
            .iter()
            .map(|tag| TagChip::new(tag.name.as_str()))
            .collect();

        let record = item.record;
        let search = format!(
            "{} {} {} {}",
            record.display_title,
            record.url,
            record.host,
            tags.iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase();

        let collection = record.collection_name.as_ref().map(|name| CollectionBadge {
            emoji: record
                .collection_emoji
                .clone()
                .unwrap_or_else(|| "📁".into()),
            name: name.clone(),
            href: record.collection_id.map_or_else(
                || "/collections".to_owned(),
                |id| format!("/collections/{id}"),
            ),
        });

        Self {
            id: record.id.get(),
            title: record.display_title,
            description: record.description,
            url: record.url,
            domain: record.host,
            pinned: record.pinned,
            has_thumbnail: record.thumbnail_content_id.is_some(),
            thumb_url: thumb_url(record.id.get()),
            collection,
            date: if record.capture_count > 0 {
                Stamp::from_ms(record.last_captured_at)
            } else {
                Stamp::missing("Never archived")
            },
            capture_count: record.capture_count,
            tags,
            search,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CaptureView {
    pub(crate) id: i64,
    pub(crate) version: i64,
    pub(crate) title: String,
    pub(crate) captured: Stamp,
    pub(crate) status: String,
    pub(crate) has_html: bool,
    pub(crate) has_screenshot: bool,
}

impl From<CaptureSummary> for CaptureView {
    fn from(capture: CaptureSummary) -> Self {
        Self {
            id: capture.id.get(),
            version: capture.version,
            title: capture
                .title
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| format!("Version {}", capture.version)),
            captured: Stamp::from_ms(capture.captured_at),
            status: capture
                .status_code
                .map_or_else(|| "—".to_owned(), |code| code.to_string()),
            has_html: capture.content_hash.is_some(),
            has_screenshot: capture.has_screenshot,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HighlightView {
    pub(crate) id: i64,
    pub(crate) text: String,
    pub(crate) green: bool,
}

impl From<HighlightRecord> for HighlightView {
    fn from(highlight: HighlightRecord) -> Self {
        Self {
            id: highlight.id,
            green: highlight.color == "green",
            text: highlight.text,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CollectionCard {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) emoji: String,
    pub(crate) description: String,
    pub(crate) color: String,
    pub(crate) public: bool,
    pub(crate) builtin: bool,
    pub(crate) link_count: i64,
}

impl From<CollectionRecord> for CollectionCard {
    fn from(record: CollectionRecord) -> Self {
        Self {
            id: record.id.get(),
            name: record.name,
            emoji: record.emoji,
            description: record.description,
            color: record.color,
            public: record.public,
            builtin: record.is_builtin,
            link_count: record.link_count,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CollectionChoice {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) emoji: String,
    pub(crate) selected: bool,
}

pub(crate) fn collection_choices(
    collections: Vec<CollectionRecord>,
    selected: Option<i64>,
) -> Vec<CollectionChoice> {
    collections
        .into_iter()
        .map(|collection| CollectionChoice {
            id: collection.id.get(),
            name: collection.name,
            emoji: collection.emoji,
            selected: selected == Some(collection.id.get()),
        })
        .collect()
}

#[derive(Debug, Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "view model flags drive template conditionals"
)]
pub(crate) struct PendingView {
    pub(crate) id: i64,
    pub(crate) url: String,
    pub(crate) status_label: &'static str,
    pub(crate) status_class: &'static str,
    pub(crate) added: Stamp,
    pub(crate) attempts: i64,
    pub(crate) last_error: Option<String>,
    pub(crate) tags: Vec<TagChip>,
    pub(crate) tags_csv: String,
    pub(crate) retryable: bool,
    pub(crate) in_progress: bool,
    pub(crate) is_queued: bool,
    pub(crate) is_failed: bool,
}

impl From<PendingResponse> for PendingView {
    fn from(item: PendingResponse) -> Self {
        let record = item.record;
        let tags: Vec<TagChip> = item
            .tags
            .iter()
            .map(|tag| TagChip::new(tag.name.as_str()))
            .collect();
        let tags_csv = tags
            .iter()
            .map(|tag| tag.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let (status_label, status_class) = match record.status {
            PendingStatus::Queued => ("Queued", "badge-info"),
            PendingStatus::InProgress => ("In progress", "badge-warning"),
            PendingStatus::Failed => ("Failed", "badge-error"),
        };

        Self {
            id: record.id.get(),
            url: record.url,
            status_label,
            status_class,
            added: Stamp::from_ms(record.created_at),
            attempts: record.attempts,
            last_error: record.last_error,
            tags,
            tags_csv,
            retryable: record.status == PendingStatus::Failed,
            in_progress: record.status == PendingStatus::InProgress,
            is_queued: record.status == PendingStatus::Queued,
            is_failed: record.status == PendingStatus::Failed,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TagRow {
    pub(crate) name: String,
    pub(crate) href: String,
    pub(crate) page_count: i64,
    pub(crate) pending_count: i64,
}

impl From<TagSummary> for TagRow {
    fn from(summary: TagSummary) -> Self {
        let name = summary.name.as_str().to_owned();
        Self {
            href: format!("/tags/{name}"),
            name,
            page_count: summary.page_count,
            pending_count: summary.pending_count,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StatCard {
    pub(crate) label: &'static str,
    pub(crate) value: i64,
    pub(crate) href: &'static str,
}

impl StatCard {
    pub(crate) const fn new(label: &'static str, value: i64, href: &'static str) -> Self {
        Self { label, value, href }
    }
}
