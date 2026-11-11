use sqlx::prelude::FromRow;

/// Base query over the `v_users` view (live users + profile, never credentials),
/// with the user's active roles folded into one comma separated column.
#[macro_export]
macro_rules! user_select {
    () => {
       "SELECT id, public_id, email, email_verified_at, username, phone_e164, \
           status, last_login_at, created_at, display_name, first_name, last_name, avatar_url, locale, timezone, \
           (SELECT group_concat(r.role) FROM v_user_roles r WHERE r.user_id = v_users.id) AS roles \
       FROM v_users"
    };
}

#[derive(Debug, FromRow)]
pub(crate) struct UserRow {
    // pub id: i64,
    pub public_id: String,
    pub email: Option<String>,
    pub email_verified_at: Option<i64>,
    pub username: Option<String>,
    pub phone_e164: Option<String>,
    pub status: String,
    pub last_login_at: Option<i64>,
    pub created_at: i64,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub roles: Option<String>,
}
