use secrecy::SecretString;
use serde::{Deserialize, Serialize};

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

const REGISTER_MESSAGE: &str =
    "If this address can be registered, we've sent an e-mail with the next steps.";

/// Body of the registration response. Deliberately carries no account data.
#[derive(Serialize)]
pub(crate) struct RegisterResponse {
    message: &'static str,
}

impl RegisterResponse {
    pub(crate) const fn new() -> Self {
        Self {
            message: REGISTER_MESSAGE,
        }
    }
}
