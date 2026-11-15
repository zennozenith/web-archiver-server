//! Shared loading helpers for the archive pages.

use serde::Deserialize;

use crate::{
    app::AppState,
    db::page::{PageField, PageItem, PageSort},
    domain::TagId,
    error::Result,
    identity::AuthUser,
    service,
    transport::web::viewmodels::archive::LinkCard,
    util::limit::Limit,
};

pub(crate) const LIST_LIMIT: Limit = Limit::from_const(48);

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LinkQuery {
    #[serde(default)]
    pub(crate) q: String,
    #[serde(default)]
    pub(crate) new: u8,
    #[serde(default)]
    pub(crate) sort: String,
    #[serde(default, rename = "in")]
    pub(crate) field: String,
    pub(crate) cursor: Option<String>,
    pub(crate) collection_id: Option<i64>,
    #[serde(default)]
    pub(crate) pinned_only: bool,
    #[serde(default)]
    pub(crate) unorganized_only: bool,
}

#[derive(Debug, Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "view model flags drive template conditionals"
)]
pub(crate) struct LinkFilters {
    pub(crate) text: String,
    /// Percent-encoded text, safe to place in a query string.
    pub(crate) text_qs: String,
    pub(crate) sort_value: &'static str,
    pub(crate) field_value: &'static str,
    pub(crate) sort_newest: bool,
    pub(crate) sort_oldest: bool,
    pub(crate) sort_name: bool,
    pub(crate) field_name: bool,
    pub(crate) field_url: bool,
    pub(crate) field_text: bool,
    pub(crate) field_tags: bool,
}

impl LinkQuery {
    fn sort(&self) -> PageSort {
        match self.sort.as_str() {
            "oldest" => PageSort::Oldest,
            "name" => PageSort::Name,
            _ => PageSort::Newest,
        }
    }

    fn field(&self) -> PageField {
        match self.field.as_str() {
            "url" => PageField::Url,
            "text" => PageField::Text,
            "tags" => PageField::Tags,
            _ => PageField::Name,
        }
    }

    pub(crate) fn filters(&self) -> LinkFilters {
        let sort = self.sort();
        let field = self.field();
        LinkFilters {
            text: self.q.clone(),
            text_qs: crate::transport::web::viewmodels::pct_encode(&self.q),
            sort_value: match sort {
                PageSort::Oldest => "oldest",
                PageSort::Name => "name",
                PageSort::Newest => "newest",
            },
            field_value: match field {
                PageField::Url => "url",
                PageField::Text => "text",
                PageField::Tags => "tags",
                PageField::Name => "name",
            },
            sort_newest: sort == PageSort::Newest,
            sort_oldest: sort == PageSort::Oldest,
            sort_name: sort == PageSort::Name,
            field_name: field == PageField::Name,
            field_url: field == PageField::Url,
            field_text: field == PageField::Text,
            field_tags: field == PageField::Tags,
        }
    }

    fn service_query(&self) -> service::page::PageListQuery {
        service::page::PageListQuery {
            text: (!self.q.trim().is_empty()).then(|| self.q.clone()),
            field: self.field(),
            sort: self.sort(),
            collection_id: self.collection_id.map(Into::into),
            unorganized_only: self.unorganized_only,
            tag_id: None,
            pinned_only: self.pinned_only,
            limit: LIST_LIMIT,
            cursor: self.cursor.as_deref().and_then(|c| c.parse().ok()),
        }
    }
}

pub(crate) fn cards(items: Vec<PageItem>) -> Vec<LinkCard> {
    items.into_iter().map(LinkCard::new).collect()
}

/// Loads one page of cards plus the cursor for the next page.
pub(crate) async fn load(
    state: &AppState,
    actor: &AuthUser,
    query: &LinkQuery,
) -> Result<(Vec<LinkCard>, Option<String>)> {
    let page = service::page::list(state, actor, &query.service_query()).await?;
    Ok((
        cards(page.items),
        page.next_cursor.map(|cursor| cursor.to_string()),
    ))
}

pub(crate) async fn load_tagged(
    state: &AppState,
    actor: &AuthUser,
    tag_id: TagId,
    limit: Limit,
) -> Result<Vec<LinkCard>> {
    let query = service::page::PageListQuery {
        tag_id: Some(tag_id),
        limit,
        ..Default::default()
    };
    Ok(cards(
        service::page::list(state, actor, &query).await?.items,
    ))
}

pub(crate) async fn load_collection(
    state: &AppState,
    actor: &AuthUser,
    collection_id: crate::db::collection::CollectionId,
    limit: Limit,
) -> Result<Vec<LinkCard>> {
    let query = service::page::PageListQuery {
        collection_id: Some(collection_id),
        limit,
        ..Default::default()
    };
    Ok(cards(
        service::page::list(state, actor, &query).await?.items,
    ))
}

pub(crate) async fn collection_choices(
    state: &AppState,
    actor: &AuthUser,
    selected: Option<i64>,
) -> Result<Vec<crate::transport::web::viewmodels::archive::CollectionChoice>> {
    let collections = service::collection::list(&state.db, actor).await?;
    Ok(crate::transport::web::viewmodels::archive::collection_choices(collections, selected))
}
