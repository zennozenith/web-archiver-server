use secrecy::SecretString;
use serde::Deserialize;

use crate::auth::LoginCredentials;

#[derive(Deserialize)]
pub(crate) struct LoginRequest {
    pub identifier: String,
    pub password: SecretString,
}

impl From<LoginRequest> for LoginCredentials {
    fn from(r: LoginRequest) -> Self {
        Self::new(&r.identifier, r.password)
    }
}
