//! Page (saved link) use-cases. Reads and mutations are owner-scoped.

use crate::{
    app::AppState,
    db::{
        capture::CaptureSummary,
        collection::{CollectionId, CollectionRecord},
        cursor::{Cursor, CursorPage},
        page::{
            self, CreatePage, HighlightRecord, PageCursor, PageDetail, PageField, PageFilter,
            PageId, PageItem, PageSort, UpdatePage,
        },
        patch::FieldPatch,
        tag::Tag,
    },
    error::{AppError, OptionNotFound as _, Resource, Result},
    identity::{AuthUser, Permission},
    infra::{TagId, TagName},
    util::limit::Limit,
};

/// Query accepted by the link listing. The owner is filled in by the service.
#[derive(Debug, Default)]
pub(crate) struct PageListQuery {
    pub text: Option<String>,
    pub field: PageField,
    pub sort: PageSort,
    pub collection_id: Option<CollectionId>,
    pub unorganized_only: bool,
    pub tag_id: Option<TagId>,
    pub pinned_only: bool,
    pub limit: Limit,
    pub cursor: Option<PageCursor>,
}

impl PageListQuery {
    fn filter(&self, owner: i64) -> PageFilter {
        PageFilter {
            owner_user_id: owner,
            text: self.text.clone(),
            field: self.field,
            sort: self.sort,
            collection_id: self.collection_id,
            unorganized_only: self.unorganized_only,
            tag_id: self.tag_id,
            pinned_only: self.pinned_only,
            limit: self.limit,
            cursor: self.cursor,
        }
    }
}

pub(crate) async fn list(
    state: &AppState,
    actor: &AuthUser,
    query: &PageListQuery,
) -> Result<CursorPage<PageItem, PageCursor>> {
    actor.require(Permission::ARCHIVE_READ)?;
    PageItem::list(state.db.reader(), &query.filter(actor.id)).await
}

pub(crate) async fn get(state: &AppState, actor: &AuthUser, id: PageId) -> Result<PageDetail> {
    actor.require(Permission::ARCHIVE_READ)?;
    PageDetail::load(state.db.reader(), actor.id, id)
        .await?
        .or_not_found(Resource::Page)
}

pub(crate) async fn lookup(state: &AppState, actor: &AuthUser, url: &str) -> Result<PageDetail> {
    actor.require(Permission::ARCHIVE_READ)?;
    let url = page::parse_url(url)?;
    PageDetail::load_by_url(state.db.reader(), actor.id, &url)
        .await?
        .or_not_found(Resource::Page)
}

pub(crate) async fn create(
    state: &AppState,
    actor: &AuthUser,
    upload: CreatePage,
) -> Result<PageItem> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    ensure_collection(state, actor, upload.collection_id).await?;
    upload.execute(&state.db, actor.id, &state.blobs).await
}

pub(crate) async fn patch(
    state: &AppState,
    actor: &AuthUser,
    id: PageId,
    update: UpdatePage,
) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;

    if let FieldPatch::Set(raw) = &update.collection_id {
        let collection_id = CollectionId::parse_opt(Some(raw.as_str()));
        ensure_collection(state, actor, collection_id).await?;
    }

    if update.apply(&state.db, actor.id, id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Page))
    }
}

pub(crate) async fn delete(state: &AppState, actor: &AuthUser, id: PageId) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if PageId::delete(id, &state.db, actor.id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Page))
    }
}

pub(crate) async fn toggle_pin(state: &AppState, actor: &AuthUser, id: PageId) -> Result<bool> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    page::toggle_pinned(state.db.writer(), actor.id, id)
        .await?
        .or_not_found(Resource::Page)
}

/// Replaces the page thumbnail. `None` clears it.
pub(crate) async fn set_thumbnail(
    state: &AppState,
    actor: &AuthUser,
    id: PageId,
    thumbnail: Option<crate::infra::Image>,
) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if id
        .set_thumbnail(&state.db, actor.id, &state.blobs, thumbnail)
        .await?
    {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Page))
    }
}

pub(crate) async fn captures(
    state: &AppState,
    actor: &AuthUser,
    id: PageId,
    after: Option<Cursor>,
    limit: Option<Limit>,
) -> Result<CursorPage<CaptureSummary>> {
    actor.require(Permission::ARCHIVE_READ)?;
    if !page::exists(state.db.reader(), actor.id, id).await? {
        return Err(AppError::NotFound(Resource::Page));
    }
    Ok(CaptureSummary::list(
        state.db.reader(),
        actor.id,
        id,
        after,
        limit.unwrap_or(Limit::from_const(50)),
    )
    .await?)
}

// --------------------------------------------------------------------- tags

pub(crate) async fn put_tags(
    state: &AppState,
    actor: &AuthUser,
    id: PageId,
    names: &[TagName],
) -> Result<Vec<Tag>> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    page::put_tags(&state.db, actor.id, id, names)
        .await?
        .or_not_found(Resource::Page)
}

pub(crate) async fn add_tag(
    state: &AppState,
    actor: &AuthUser,
    id: PageId,
    name: &TagName,
) -> Result<Tag> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    page::add_tag(&state.db, actor.id, id, name)
        .await?
        .or_not_found(Resource::Page)
}

pub(crate) async fn remove_tag(
    state: &AppState,
    actor: &AuthUser,
    id: PageId,
    tag_id: TagId,
) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if page::remove_tag(&state.db, actor.id, id, tag_id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::PageTag))
    }
}

// --------------------------------------------------------------- highlights

pub(crate) async fn highlights(
    state: &AppState,
    actor: &AuthUser,
    id: PageId,
) -> Result<Vec<HighlightRecord>> {
    actor.require(Permission::ARCHIVE_READ)?;
    Ok(HighlightRecord::list(state.db.reader(), actor.id, id).await?)
}

pub(crate) async fn add_highlight(
    state: &AppState,
    actor: &AuthUser,
    id: PageId,
    text: &str,
    color: Option<&str>,
) -> Result<HighlightRecord> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::invalid(
            "empty_text",
            "a highlight needs some text",
        ));
    }
    let color = color
        .map(str::trim)
        .filter(|color| !color.is_empty())
        .unwrap_or("yellow");
    if !matches!(color, "yellow" | "green") {
        return Err(AppError::invalid(
            "invalid_color",
            "highlight color must be yellow or green",
        ));
    }

    if !page::exists(state.db.writer(), actor.id, id).await? {
        return Err(AppError::NotFound(Resource::Page));
    }
    Ok(HighlightRecord::add(state.db.writer(), actor.id, id, text, color).await?)
}

pub(crate) async fn update_highlight(
    state: &AppState,
    actor: &AuthUser,
    id: i64,
    text: Option<&str>,
    color: Option<&str>,
) -> Result<HighlightRecord> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    HighlightRecord::update(&state.db, actor.id, id, text, color)
        .await?
        .or_not_found(Resource::Highlight)
}

pub(crate) async fn delete_highlight(state: &AppState, actor: &AuthUser, id: i64) -> Result<()> {
    actor.require(Permission::ARCHIVE_WRITE)?;
    if HighlightRecord::delete(&state.db, actor.id, id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound(Resource::Highlight))
    }
}

async fn ensure_collection(
    state: &AppState,
    actor: &AuthUser,
    collection_id: Option<CollectionId>,
) -> Result<()> {
    if let Some(collection_id) = collection_id
        && !CollectionRecord::exists(state.db.writer(), actor.id, collection_id).await?
    {
        return Err(AppError::invalid(
            "invalid_collection",
            "that collection does not exist",
        ));
    }
    Ok(())
}
