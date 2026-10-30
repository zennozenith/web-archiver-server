use axum::{extract::State, http::StatusCode};
use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

use crate::{
    db::helpers::load_tag,
    dto::{CreateTag, MergeTag, PatchTag},
    error::{AppError, OptionExt, Result},
    extract::{ApiJson, ApiPath},
    models::TagRow,
    tag::clean_tag_name,
};

/// Colors are palette keys like "red" or "deep-blue", never hex values.
pub(super) fn validate_color(color: Option<&str>) -> Result<Option<String>> {
    let Some(c) = color.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    let ok = c.len() <= 32
        && c.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(AppError::invalid(
            "invalid_color",
            "color must be a palette key: lowercase letters, digits and '-' (max 32)",
        ));
    }
    Ok(Some(c.to_owned()))
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(super) struct TagWithCount {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub created_at: i64,
    pub pages: i64,
    pub pending: i64,
}

// GET /api/tags
pub(super) async fn list_tags(
    State(pool): State<SqlitePool>,
) -> Result<ApiJson<Vec<TagWithCount>>> {
    let rows = sqlx::query_as(
        "SELECT t.id, t.name, t.color, t.created_at,
                (SELECT COUNT(*) FROM url_tag ut WHERE ut.tag_id = t.id)     AS pages,
                (SELECT COUNT(*) FROM pending_tag pt WHERE pt.tag_id = t.id) AS pending
         FROM tag t
         ORDER BY pages DESC, t.name COLLATE NOCASE",
    )
    .fetch_all(&pool)
    .await?;
    Ok(ApiJson(rows))
}

// POST /api/tags
pub(super) async fn create_tag(
    State(pool): State<SqlitePool>,
    ApiJson(req): ApiJson<CreateTag>,
) -> Result<(StatusCode, ApiJson<TagRow>)> {
    let name = clean_tag_name(&req.name)?;
    let color = validate_color(req.color.as_deref())?;

    let created: Option<TagRow> = sqlx::query_as(
        "INSERT INTO tag (name, color) VALUES (?, ?)
         ON CONFLICT(name) DO NOTHING
         RETURNING id, name, color, created_at",
    )
    .bind(&name)
    .bind(&color)
    .fetch_optional(&pool)
    .await?;

    if let Some(tag) = created {
        Ok((StatusCode::CREATED, ApiJson(tag)))
    } else {
        let existing_id: i64 = sqlx::query_scalar("SELECT id FROM tag WHERE name = ?")
            .bind(&name)
            .fetch_one(&pool)
            .await?;
        Err(AppError::DuplicateTag { existing_id })
    }
}

// PATCH /api/tags/{id}   (color: null clears, absent keeps)
pub(super) async fn patch_tag(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(req): ApiJson<PatchTag>,
) -> Result<ApiJson<TagRow>> {
    let mut tx = pool.begin().await?;
    let current = load_tag(&mut tx, id).await?.or_not_found("tag")?;

    let name = match req.name.as_deref() {
        Some(n) => clean_tag_name(n)?,
        None => current.name.into(),
    };
    let color = match &req.color {
        None => current.color.clone(),
        Some(c) => validate_color(c.as_deref())?,
    };

    let clash: Option<i64> = sqlx::query_scalar("SELECT id FROM tag WHERE name = ? AND id != ?")
        .bind(&name)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(existing_id) = clash {
        return Err(AppError::DuplicateTag { existing_id });
    }

    sqlx::query("UPDATE tag SET name = ?, color = ? WHERE id = ?")
        .bind(&name)
        .bind(&color)
        .bind(id)
        .execute(&mut *tx)
        .await?;

    let updated = load_tag(&mut tx, id).await?.or_not_found("tag")?;
    tx.commit().await?;
    Ok(ApiJson(updated))
}

// DELETE /api/tags/{id}   (url_tag / pending_tag rows cascade)
pub(super) async fn delete_tag(
    State(pool): State<SqlitePool>,
    ApiPath(id): ApiPath<i64>,
) -> Result<StatusCode> {
    let n = sqlx::query("DELETE FROM tag WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound("tag"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// POST /api/tags/{id}/merge   { "into": <tag_id> }
pub(super) async fn merge_tag(
    State(pool): State<SqlitePool>,
    ApiPath(from): ApiPath<i64>,
    ApiJson(req): ApiJson<MergeTag>,
) -> Result<ApiJson<TagRow>> {
    let into = req.into;
    if from == into {
        return Err(AppError::BadRequest(
            "cannot merge a tag into itself".into(),
        ));
    }

    let mut tx = pool.begin().await?;

    let found: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tag WHERE id IN (?, ?)")
        .bind(from)
        .bind(into)
        .fetch_one(&mut *tx)
        .await?;
    if found != 2 {
        return Err(AppError::NotFound("tag"));
    }

    sqlx::query("INSERT OR IGNORE INTO url_tag (url_id, tag_id) SELECT url_id, ? FROM url_tag WHERE tag_id = ?")
        .bind(into)
        .bind(from)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO pending_tag (pending_id, tag_id) SELECT pending_id, ? FROM pending_tag WHERE tag_id = ?",
    )
    .bind(into)
    .bind(from)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM tag WHERE id = ?")
        .bind(from)
        .execute(&mut *tx)
        .await?;

    let target = load_tag(&mut tx, into).await?.or_not_found("tag")?;
    tx.commit().await?;
    Ok(ApiJson(target))
}
