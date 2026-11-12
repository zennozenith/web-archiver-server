use sqlx::{Result, SqliteExecutor};

use crate::auth::{ProfileUpdate, ProfileView};

pub(crate) async fn find<'e>(ex: impl SqliteExecutor<'e>, user_id: i64) -> Result<ProfileView> {
    sqlx::query_as(
        "SELECT display_name, first_name, last_name, avatar_url, bio, birth_date,
                locale, timezone, updated_at
           FROM user_profiles WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_one(ex)
    .await
}

/// `updated_at` is maintained by `trg_profiles_updated_at`.
pub(crate) async fn update<'e>(
    ex: impl SqliteExecutor<'e>,
    user_id: i64,
    p: &ProfileUpdate,
) -> Result<()> {
    sqlx::query(
        "UPDATE user_profiles
            SET display_name = ?, first_name = ?, last_name = ?, bio = ?, birth_date = ?,
                locale   = COALESCE(?, locale),
                timezone = COALESCE(?, timezone)
          WHERE user_id = ?",
    )
    .bind(p.display_name.as_deref())
    .bind(p.first_name.as_deref())
    .bind(p.last_name.as_deref())
    .bind(p.bio.as_deref())
    .bind(p.birth_date.as_deref())
    .bind(p.locale.as_deref())
    .bind(p.timezone.as_deref())
    .bind(user_id)
    .execute(ex)
    .await?;
    Ok(())
}
