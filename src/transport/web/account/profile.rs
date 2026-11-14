use askama::Template;
use axum::extract::State;

use crate::{
    AppState,
    identity::{AuthUser, Permission},
    prelude::*,
    service::me,
    transport::{
        dto::user::ProfileRequest,
        extract::body::AppForm,
        htmx::{Fragments, Render, ToastKind},
        web::{
            account::overview::{AccountOverview, Mine},
            viewmodels::{Section, Shell, UserItem},
        },
    },
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
}

#[derive(Debug, Template)]
#[template(path = "profile.templ.html")]
pub(crate) struct ProfileTpl {
    shell: Shell,
    me: UserItem,
    mine: Mine,
    profile: ProfileFields,
    can_edit: bool,
}

pub(super) async fn page(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Render<ProfileTpl>> {
    let data = me::get_me(&state.db, &user).await?;
    let me = UserItem::new(data.mine.user.clone(), &user.public_id);

    let p = data.profile;
    let mine = Mine {
        last_login: me.last_login,
        sessions: data.mine.sessions,
        api_keys: data.mine.api_keys,
    };

    Ok(Render(ProfileTpl {
        shell: Shell::new::<AccountOverview>(&user, Section::Profile),
        me: UserItem::new(data.mine.user, &user.public_id),
        mine,
        profile: ProfileFields {
            display_name: p.display_name.unwrap_or_default(),
            first_name: p.first_name.unwrap_or_default(),
            last_name: p.last_name.unwrap_or_default(),
            birth_date: p.birth_date.unwrap_or_default(),
            bio: p.bio.unwrap_or_default(),
            locale: p.locale,
            timezone: p.timezone,
        },
        can_edit: user.can(Permission::PROFILE_WRITE),
    }))
}

/// The form posts every field, so it shares the API's request type.
pub(super) async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    AppForm(form): AppForm<ProfileRequest>,
) -> Result<Fragments> {
    me::update_profile(&state.db, &user, form.into()).await?;
    Fragments::new().toast(ToastKind::Success, "Profile saved")
}
