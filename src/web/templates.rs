use askama::Template;

use crate::{
    db::page::{PageCursor, PageStats},
    web::view::{CollectionOption, CollectionView, FormatView, Page, PageView, Section, TagView},
};

// ───────────────────────────────────────────────────────────────────────────
// Full pages
// ───────────────────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "library/dashboard.html")]
pub(crate) struct DashboardTemplate {
    pub page: Page,
    pub stats: PageStats,
    pub sections: Vec<Section>,
}

#[derive(Template)]
#[template(path = "library/links.html")]
pub(crate) struct PagesTemplate {
    pub page: Page,
    pub links: Vec<PageView>,
    pub next_cursor: Option<PageCursor>,
}

#[derive(Template)]
#[template(path = "library/pinned.html")]
pub(crate) struct PinnedTemplate {
    pub page: Page,
    pub links: Vec<PageView>,
}

#[derive(Template)]
#[template(path = "library/collections.html")]
pub(crate) struct CollectionsTemplate {
    pub page: Page,
    pub collections: Vec<CollectionView>,
}

#[derive(Template)]
#[template(path = "library/collection.html")]
pub(crate) struct CollectionTemplate {
    pub page: Page,
    pub collection: CollectionView,
    pub links: Vec<PageView>,
}

#[derive(Template)]
#[template(path = "library/collection_form.html")]
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
#[template(path = "library/link_detail.html")]
pub(crate) struct PageDetailTemplate {
    pub page: Page,
    pub link: PageView,
    pub collections: Vec<CollectionOption>,
    pub edit: bool,
}

#[derive(Template)]
#[template(path = "library/tags.html")]
pub(crate) struct TagsTemplate {
    pub page: Page,
    pub tags: Vec<TagView>,
}

#[derive(Template)]
#[template(path = "library/tag.html")]
pub(crate) struct TagTemplate {
    pub page: Page,
    pub tag: TagView,
    pub links: Vec<PageView>,
}

#[derive(Template)]
#[template(path = "library/public_collection.html")]
pub(crate) struct PublicCollectionTemplate {
    pub page: Page,
    pub collection: CollectionView,
    pub links: Vec<PageView>,
}

#[derive(Template)]
#[template(path = "library/link_archive.html")]
pub(crate) struct ArchiveTemplate {
    pub page: Page,
    pub link: PageView,
    pub format: FormatView,
}

// ───────────────────────────────────────────────────────────────────────────
// HTMX fragments
// ───────────────────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "library/_link_grid.html")]
pub(crate) struct PageGridTemplate {
    pub links: Vec<PageView>,
}

#[derive(Template)]
#[template(path = "library/_link_card.html")]
pub(crate) struct PageCardTemplate {
    pub link: PageView,
}

#[derive(Template)]
#[template(path = "library/_link_content.html")]
pub(crate) struct PageContentTemplate {
    pub link: PageView,
    pub format: FormatView,
    pub readable: bool,
}

#[derive(Template)]
#[template(path = "library/_formats.html")]
pub(crate) struct FormatsTemplate {
    pub link: PageView,
}

#[derive(Template)]
#[template(path = "library/_tag_list.html")]
pub(crate) struct TagListTemplate {
    pub tags: Vec<TagView>,
}
