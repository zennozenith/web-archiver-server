//! Askama template contexts used by the Linkwarden-style UI.

use askama::Template;

use crate::{
    db::link::LinkStats,
    web::view::{CollectionOption, CollectionView, FormatView, LinkView, Page, Section, TagView},
};

// ───────────────────────────────────────────────────────────────────────────
// Full pages
// ───────────────────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "dashboard.html")]
pub(crate) struct DashboardTemplate {
    pub page: Page,
    pub stats: LinkStats,
    pub sections: Vec<Section>,
}

#[derive(Template)]
#[template(path = "links.html")]
pub(crate) struct LinksTemplate {
    pub page: Page,
    pub links: Vec<LinkView>,
    pub has_more: bool,
    pub next_page: usize,
}

#[derive(Template)]
#[template(path = "pinned.html")]
pub(crate) struct PinnedTemplate {
    pub page: Page,
    pub links: Vec<LinkView>,
}

#[derive(Template)]
#[template(path = "collections.html")]
pub(crate) struct CollectionsTemplate {
    pub page: Page,
    pub collections: Vec<CollectionView>,
}

#[derive(Template)]
#[template(path = "collection.html")]
pub(crate) struct CollectionTemplate {
    pub page: Page,
    pub collection: CollectionView,
    pub links: Vec<LinkView>,
}

#[derive(Template)]
#[template(path = "collection_form.html")]
pub(crate) struct CollectionFormTemplate {
    pub page: Page,
    pub editing: bool,
    pub form_title: String,
    pub submit_label: String,
    pub form_action: String,
    pub name: String,
    pub description: String,
    pub color: String,
    pub collections: Vec<CollectionOption>,
}

#[derive(Template)]
#[template(path = "link_detail.html")]
pub(crate) struct LinkDetailTemplate {
    pub page: Page,
    pub link: LinkView,
    pub collections: Vec<CollectionOption>,
    pub edit: bool,
}

#[derive(Template)]
#[template(path = "tags.html")]
pub(crate) struct TagsTemplate {
    pub page: Page,
    pub tags: Vec<TagView>,
}

#[derive(Template)]
#[template(path = "tag.html")]
pub(crate) struct TagTemplate {
    pub page: Page,
    pub tag: TagView,
    pub links: Vec<LinkView>,
}

#[derive(Template)]
#[template(path = "public_collection.html")]
pub(crate) struct PublicCollectionTemplate {
    pub page: Page,
    pub collection: CollectionView,
    pub links: Vec<LinkView>,
}

#[derive(Template)]
#[template(path = "archive.html")]
pub(crate) struct ArchiveTemplate {
    pub page: Page,
    pub link: LinkView,
    pub format: FormatView,
}

// ───────────────────────────────────────────────────────────────────────────
// HTMX fragments
// ───────────────────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "partials/link_grid.html")]
pub(crate) struct LinkGridTemplate {
    pub links: Vec<LinkView>,
}

#[derive(Template)]
#[template(path = "partials/link_card.html")]
pub(crate) struct LinkCardTemplate {
    pub link: LinkView,
}

#[derive(Template)]
#[template(path = "partials/link_content.html")]
pub(crate) struct LinkContentTemplate {
    pub link: LinkView,
    pub format: FormatView,
    pub readable: bool,
}

#[derive(Template)]
#[template(path = "partials/formats.html")]
pub(crate) struct FormatsTemplate {
    pub link: LinkView,
}

#[derive(Template)]
#[template(path = "partials/tag_list.html")]
pub(crate) struct TagListTemplate {
    pub tags: Vec<TagView>,
}
