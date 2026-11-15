use askama::Template;
use axum::extract::State;
use secrecy::ExposeSecret as _;

use crate::{
    AppState,
    error::AppError,
    identity::{ApiKeyView, AuthUser, NewApiKey, Permission},
    prelude::*,
    service::me,
    transport::{
        extract::body::{AppForm, AppPath},
        htmx::{Fragments, Render, ToastKind},
        web::{
            account::overview::AccountOverview,
            viewmodels::{Section, Shell, Stamp},
        },
    },
};

/// Pre-ticked in the "new key" form.
const DEFAULT_SCOPE: &str = Permission::PROFILE_READ.as_str();
/// The wildcard scope, flagged as dangerous in the UI.
const WILDCARD: &str = Permission::WILDCARD.as_str();

#[derive(Debug)]
pub(crate) struct KeyItem {
    pub(crate) public_id: String,
    pub(crate) name: String,
    pub(crate) prefix: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) last_used: Stamp,
    pub(crate) expires: Stamp,
}

impl From<ApiKeyView> for KeyItem {
    fn from(k: ApiKeyView) -> Self {
        Self {
            scopes: k.scopes(),
            last_used: Stamp::from_opt_ms(k.last_used_at, "Never"),
            expires: Stamp::from_opt_ms(k.expires_at, "Never"),
            public_id: k.public_id,
            name: k.name,
            prefix: k.key_prefix,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ScopeOption {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) checked: bool,
    pub(crate) broad: bool,
}

async fn load_keys(state: &AppState, user: &AuthUser) -> Result<Vec<KeyItem>, AppError> {
    Ok(me::list_api_keys(&state.db, user)
        .await?
        .into_iter()
        .map(KeyItem::from)
        .collect())
}

#[derive(Debug, Template)]
#[template(path = "api_keys.templ.html")]
pub(crate) struct KeysTpl {
    shell: Shell,
    keys: Vec<KeyItem>,
    scopes: Vec<ScopeOption>,
}

#[derive(Debug, Template)]
#[template(path = "partials/key_rows.templ.html")]
struct KeyRowsTpl {
    keys: Vec<KeyItem>,
}

/// Shown once, right after a key is created.
#[derive(Debug, Template)]
#[template(path = "partials/new_key.templ.html")]
struct NewKeyTpl {
    key: String,
}

pub(super) async fn page(State(state): State<AppState>, user: AuthUser) -> Result<Render<KeysTpl>> {
    let scopes = me::available_scopes(&state.db, &user)
        .await?
        .into_iter()
        .map(|p| ScopeOption {
            checked: p.name == DEFAULT_SCOPE,
            broad: p.name == WILDCARD,
            description: p.description.unwrap_or_default(),
            name: p.name,
        })
        .collect();
    Ok(Render(KeysTpl {
        shell: Shell::new::<AccountOverview>(&user, Section::Keys),
        keys: load_keys(&state, &user).await?,
        scopes,
    }))
}

/// Checkboxes repeat the `scopes` field, which a struct cannot collect, so the
/// body is read as key/value pairs.
#[derive(Debug, Default)]
struct KeyForm {
    name: String,
    days: Option<i64>,
    scopes: Vec<String>,
}

impl From<Vec<(String, String)>> for KeyForm {
    fn from(pairs: Vec<(String, String)>) -> Self {
        let mut form = Self::default();
        for (field, value) in pairs {
            match field.as_str() {
                "name" => form.name = value,
                "days" => form.days = value.trim().parse().ok(),
                "scopes" => form.scopes.push(value),
                _ => {}
            }
        }
        form
    }
}

pub(super) async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    AppForm(pairs): AppForm<Vec<(String, String)>>,
) -> Result<Fragments> {
    let form = KeyForm::from(pairs);
    let request = NewApiKey::new(&form.name, form.scopes, form.days)?;
    let created = me::create_api_key(&state.db, &user, request).await?;

    Fragments::new()
        .push(&KeyRowsTpl {
            keys: load_keys(&state, &user).await?,
        })?
        .push(&NewKeyTpl {
            key: created.key.expose_secret().to_owned(),
        })?
        .toast(ToastKind::Success, "API key created")
}

pub(super) async fn revoke(
    State(state): State<AppState>,
    user: AuthUser,
    AppPath(public_id): AppPath<String>,
) -> Result<Fragments> {
    me::revoke_api_key(&state.db, &user, &public_id).await?;
    Fragments::new()
        .push(&KeyRowsTpl {
            keys: load_keys(&state, &user).await?,
        })?
        .toast(ToastKind::Success, "API key revoked")
}
