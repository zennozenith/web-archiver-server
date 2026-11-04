use askama::Template;
use sqlx::SqlitePool;

use crate::{
    db::{page, tag::Tag},
    error::Result,
};

pub(super) struct CaptureView {
    pub id: i64,
    pub version: i64,
    pub title: String,
    pub captured_at: String,
    pub status: String,
    pub has_html: bool,
    pub has_screenshot: bool,
}

#[derive(Template)]
#[template(path = "archive/_page_card.html")]
pub(super) struct PageCardTemplate {
    pub id: i64,
    pub url: String,
    pub host: String,
    pub display_title: String,
    pub search: String,
    pub last_captured: String,
    pub capture_count: i64,
    pub version_label: String,
    pub tags: Vec<Tag>,
    pub captures: Vec<CaptureView>,
}

#[derive(Template)]
#[template(path = "archive/_page_tags.html")]
pub(super) struct PageTagsTemplate {
    pub page_id: i64,
    pub tags: Vec<Tag>,
}

impl PageTagsTemplate {
    #[expect(dead_code, reason = "Might be needed later")]
    pub(super) async fn load(pool: &SqlitePool, page_id: i64) -> Result<Self> {
        let tags = page::tags_of(pool, page_id.into()).await?;
        Ok(Self { page_id, tags })
    }
}

// #[derive(Deserialize)]
// pub(super) struct TagForm {
//     pub name: String,
// }

// pub(super) async fn add_page_tag(
//     State(pool): State<SqlitePool>,
//     Path(id): Path<PageId>,
//     Form(form): Form<TagForm>,
// ) -> Result<HtmxResponse> {
//     let name = TagName::from_str(&form.name)?;

//     if !page::exists(&pool, id).await? {
//         return Err(AppError::NotFound("page"));
//     }

//     let mut tx = pool.begin().await?;
//     let tag_id = Tag::get_or_create_id(&mut *tx, &name).await?;
//     sqlx::query("INSERT OR IGNORE INTO page_tag (page_id, tag_id) VALUES (?, ?)")
//         .bind(id)
//         .bind(tag_id)
//         .execute(&mut *tx)
//         .await?;
//     tx.commit().await?;

//     let tags = PageTagsTemplate::load(&pool, id.get()).await?;
//     Ok(HtmxResponse::partials([PartialSwap::tpl(
//         format!("#page-tags-{id}"),
//         &tags,
//     )?
//     .swap("outerHTML")]))
// }

// pub(super) async fn remove_page_tag(
//     State(pool): State<SqlitePool>,
//     Path((id, tag_id)): Path<(i64, i64)>,
// ) -> Result<HtmxResponse> {
//     sqlx::query("DELETE FROM page_tag WHERE page_id = ? AND tag_id = ?")
//         .bind(id)
//         .bind(tag_id)
//         .execute(&pool)
//         .await?;

//     let tags = PageTagsTemplate::load(&pool, id).await?;
//     Ok(HtmxResponse::partials([PartialSwap::tpl(
//         format!("#page-tags-{id}"),
//         &tags,
//     )?
//     .swap("outerHTML")]))
// }

// pub(super) async fn delete_page(
//     State(pool): State<SqlitePool>,
//     Path(id): Path<PageId>,
// ) -> Result<HtmxResponse> {
//     // Idempotent: the card should disappear even if the page is already gone.
//     DeletePage::execute(&pool, id).await?;

//     Ok(HtmxResponse::partials([
//         PartialSwap::new(format!("#page-{id}"), "").swap("outerHTML"),
//         PartialSwap::tpl("#page-feedback", &FeedbackTemplate::success("Page deleted"))?
//             .swap("innerHTML"),
//     ]))
// }
