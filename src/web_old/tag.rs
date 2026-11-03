use askama::Template;

use crate::tag::{TagId, TagName};

#[derive(Clone)]
pub(super) struct TagView {
    pub id: TagId,
    pub name: TagName,
    pub pages: i64,
    pub pending: i64,
}

#[derive(Template)]
#[template(path = "fragments/tag-list.html")]
pub(super) struct TagListTemplate {
    pub tags: Vec<TagView>,
}

// #[derive(Deserialize)]
// pub(super) struct CreateTagForm {
//     pub name: String,
//     pub color: Option<String>,
// }

// #[derive(Deserialize)]
// pub(super) struct RenameTagForm {
//     pub name: String,
// }

// async fn tags_response(pool: &SqlitePool, message: &'static str) -> Result<HtmxResponse> {
//     let tags = TagSummary::list(pool)
//         .await?
//         .into_iter()
//         .map(|tag| TagView {
//             id: tag.id,
//             name: tag.name,
//             pages: tag.page_count,
//             pending: tag.pending_count,
//         })
//         .collect();

//     Ok(HtmxResponse::partials([
//         PartialSwap::tpl("#tags-list", &TagListTemplate { tags })?.swap("innerHTML"),
//         PartialSwap::tpl("#tags-feedback", &FeedbackTemplate::success(message))?.swap("innerHTML"),
//     ]))
// }

// pub(super) async fn create_tag(
//     State(pool): State<SqlitePool>,
//     Form(form): Form<CreateTagForm>,
// ) -> Result<HtmxResponse> {
//     let name = TagName::from_str(&form.name)?;
//     let color = parse_optional_color(form.color.as_deref())?;

//     if let CreateOutcome::Existing(existing_tag) = Tag::create(&pool, &name, color.as_ref()).await?
//     {
//         return Err(AppError::Duplicate {
//             existing_id: existing_tag.id.get(),
//             msg: "Tag exists",
//         });
//     }

//     tags_response(&pool, "Tag added").await
// }

// pub(super) async fn rename_tag(
//     State(pool): State<SqlitePool>,
//     Path(id): Path<i64>,
//     Form(form): Form<RenameTagForm>,
// ) -> Result<HtmxResponse> {
//     let name = TagName::from_str(&form.name)?;

//     if let Some(existing_id) = Tag::find_id_by_name(&pool, &name, Some(id.into())).await? {
//         return Err(AppError::Duplicate {
//             existing_id: existing_id.get(),
//             msg: "Tag exists",
//         });
//     }
//     if Tag::rename(&pool, id.into(), &name).await?.is_none() {
//         return Err(AppError::NotFound("tag"));
//     }

//     tags_response(&pool, "Tag renamed").await
// }

// pub(super) async fn delete_tag(
//     State(pool): State<SqlitePool>,
//     Path(id): Path<i64>,
// ) -> Result<HtmxResponse> {
//     if !Tag::delete(&pool, id.into()).await? {
//         return Err(AppError::NotFound("tag"));
//     }
//     tags_response(&pool, "Tag deleted").await
// }
