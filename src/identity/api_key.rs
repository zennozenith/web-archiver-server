use crate::{identity::ApiKeyToken, prelude::*, util::time::days_ms};

const NAME_MAX: usize = 100;
const MAX_EXPIRY_DAYS: i64 = 3650;

/// A validated request to mint a key. Whether the *caller* may grant these
/// scopes is the service's decision (it needs the caller).
#[derive(Debug)]
pub(crate) struct NewApiKey {
    name: String,
    scopes: Vec<String>,
    expires_in_days: Option<i64>,
}

impl NewApiKey {
    pub(crate) fn new(
        name: &str,
        scopes: Vec<String>,
        expires_in_days: Option<i64>,
    ) -> Result<Self> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::BadRequest("name is required".into()));
        }
        if name.chars().count() > NAME_MAX {
            return Err(AppError::BadRequest(format!(
                "name must be at most {NAME_MAX} characters"
            )));
        }

        let mut scopes: Vec<String> = scopes
            .into_iter()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        scopes.sort_unstable();
        scopes.dedup();
        if scopes.is_empty() {
            return Err(AppError::BadRequest(
                "at least one scope is required".into(),
            ));
        }

        if let Some(days) = expires_in_days
            && !(1..=MAX_EXPIRY_DAYS).contains(&days)
        {
            return Err(AppError::BadRequest(format!(
                "expires_in_days must be between 1 and {MAX_EXPIRY_DAYS}"
            )));
        }

        Ok(Self {
            name: name.to_owned(),
            scopes,
            expires_in_days,
        })
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn scopes(&self) -> &[String] {
        &self.scopes
    }

    pub(crate) fn scopes_json(&self) -> Result<String> {
        serde_json::to_string(&self.scopes).map_err(AppError::internal)
    }

    pub(crate) fn expires_at(&self, now: i64) -> Option<i64> {
        self.expires_in_days
            .map(|days| now.saturating_add(days_ms(days)))
    }
}

/// The plaintext key exists only here, once. Only its hash is stored.
pub(crate) struct CreatedApiKey {
    pub public_id: String,
    pub key: ApiKeyToken,
}
