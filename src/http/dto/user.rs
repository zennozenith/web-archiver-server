//! Wire formats for /me. `UserDto` is shared with the admin API.

use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};

use crate::{
    auth::{
        AccountStatus, ApiKeyView, CreatedApiKey, NewApiKey, ProfileUpdate, ProfileView,
        SessionView, UserView,
    },
    error::AppError,
    service::me::Me,
    util::time::{ms_to_dt, opt_ms_to_dt},
};

/// Public representation: the integer `id` is never exposed, only `public_id`.
#[derive(Debug, Serialize)]
pub(crate) struct UserDto {
    pub public_id: String,
    pub email: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub username: Option<String>,
    pub phone_e164: Option<String>,
    pub status: AccountStatus,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub roles: Vec<String>,
}

impl From<UserView> for UserDto {
    fn from(u: UserView) -> Self {
        let roles = u.roles();
        Self {
            public_id: u.public_id,
            email: u.email,
            email_verified_at: opt_ms_to_dt(u.email_verified_at),
            username: u.username,
            phone_e164: u.phone_e164,
            status: u.status,
            last_login_at: opt_ms_to_dt(u.last_login_at),
            created_at: ms_to_dt(u.created_at),
            display_name: u.display_name,
            first_name: u.first_name,
            last_name: u.last_name,
            avatar_url: u.avatar_url,
            locale: u.locale,
            timezone: u.timezone,
            roles,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ProfileDto {
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub birth_date: Option<String>,
    pub locale: String,
    pub timezone: String,
    pub updated_at: DateTime<Utc>,
}

impl From<ProfileView> for ProfileDto {
    fn from(p: ProfileView) -> Self {
        Self {
            display_name: p.display_name,
            first_name: p.first_name,
            last_name: p.last_name,
            avatar_url: p.avatar_url,
            bio: p.bio,
            birth_date: p.birth_date,
            locale: p.locale,
            timezone: p.timezone,
            updated_at: ms_to_dt(p.updated_at),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct MeDto {
    user: UserDto,
    profile: ProfileDto,
    permissions: Vec<String>,
}

impl From<Me> for MeDto {
    fn from(me: Me) -> Self {
        Self {
            user: me.user.into(),
            profile: me.profile.into(),
            permissions: me.permissions,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct SessionDto {
    public_id: String,
    ip_address: Option<String>,
    user_agent: Option<String>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    current: bool,
}

impl From<SessionView> for SessionDto {
    fn from(s: SessionView) -> Self {
        Self {
            public_id: s.public_id,
            ip_address: s.ip_address,
            user_agent: s.user_agent,
            created_at: ms_to_dt(s.created_at),
            last_used_at: opt_ms_to_dt(s.last_used_at),
            expires_at: ms_to_dt(s.expires_at),
            current: s.current,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct ApiKeyDto {
    public_id: String,
    name: String,
    key_prefix: String,
    scopes: Vec<String>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}

impl From<ApiKeyView> for ApiKeyDto {
    fn from(k: ApiKeyView) -> Self {
        let scopes = k.scopes();
        Self {
            public_id: k.public_id,
            name: k.name,
            key_prefix: k.key_prefix,
            scopes,
            created_at: ms_to_dt(k.created_at),
            last_used_at: opt_ms_to_dt(k.last_used_at),
            expires_at: opt_ms_to_dt(k.expires_at),
        }
    }
}

/// The only response that ever contains the plaintext key.
#[derive(Serialize)]
pub(crate) struct CreatedApiKeyDto {
    public_id: String,
    key: String,
}

impl From<CreatedApiKey> for CreatedApiKeyDto {
    fn from(c: CreatedApiKey) -> Self {
        Self {
            public_id: c.public_id,
            key: c.key.expose_secret().to_owned(),
        }
    }
}

// --------------------------------------------------------------- requests

#[derive(Deserialize)]
pub(crate) struct ProfileRequest {
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub bio: Option<String>,
    /// YYYY-MM-DD
    pub birth_date: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
}

impl From<ProfileRequest> for ProfileUpdate {
    fn from(r: ProfileRequest) -> Self {
        Self {
            display_name: r.display_name,
            first_name: r.first_name,
            last_name: r.last_name,
            bio: r.bio,
            birth_date: r.birth_date,
            locale: r.locale,
            timezone: r.timezone,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct CreateKeyRequest {
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_in_days: Option<i64>,
}

impl TryFrom<CreateKeyRequest> for NewApiKey {
    type Error = AppError;

    fn try_from(r: CreateKeyRequest) -> Result<Self, Self::Error> {
        Self::new(&r.name, r.scopes, r.expires_in_days)
    }
}
