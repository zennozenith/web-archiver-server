use askama::Template;
use axum::extract::State;

use crate::{
    AppState,
    auth::Permission,
    http::{
        dto::user::ProfileRequest,
        extract::web_user::WebUser,
        htmx::{ActionError, Fragments, PageError, Render, ToastKind},
        web::{
            form::ActionForm,
            view::{Section, Shell, Stamp, UserItem},
        },
    },
    service::me,
};

#[derive(Debug)]
pub(crate) struct ProfileFields {
    pub(crate) display_name: String,
    pub(crate) first_name: String,
    pub(crate) last_name: String,
    pub(crate) birth_date: String,
    pub(crate) bio: String,
    pub(crate) locale: String,
    pub(crate) timezone: String,
    pub(crate) updated: Stamp,
}

#[derive(Debug, Template)]
#[template(path = "profile.templ.html")]
pub(crate) struct ProfileTpl {
    shell: Shell,
    me: UserItem,
    profile: ProfileFields,
    permissions: Vec<String>,
    can_edit: bool,
}

pub(super) async fn page(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
) -> Result<Render<ProfileTpl>, PageError> {
    let data = me::get_me(&ctx, &user).await?;
    let p = data.profile;
    Ok(Render(ProfileTpl {
        shell: Shell::new(&user, Section::Profile),
        me: UserItem::new(data.user, &user.public_id),
        profile: ProfileFields {
            display_name: p.display_name.unwrap_or_default(),
            first_name: p.first_name.unwrap_or_default(),
            last_name: p.last_name.unwrap_or_default(),
            birth_date: p.birth_date.unwrap_or_default(),
            bio: p.bio.unwrap_or_default(),
            updated: Stamp::from_ms(p.updated_at),
            locale: p.locale,
            timezone: p.timezone,
        },
        permissions: data.permissions,
        can_edit: user.can(Permission::PROFILE_WRITE),
    }))
}

/// The form posts every field, so it shares the API's request type.
pub(super) async fn update(
    State(ctx): State<AppState>,
    WebUser(user): WebUser,
    ActionForm(form): ActionForm<ProfileRequest>,
) -> Result<Fragments, ActionError> {
    me::update_profile(&ctx, &user, form.into()).await?;
    Ok(Fragments::new().toast(ToastKind::Success, "Profile saved")?)
}
