use argon2::{Argon2, PasswordHash, PasswordHasher as _, PasswordVerifier as _};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest as _, Sha256};

use crate::error::AppError;

// ------------------------------------------------------------------- tokens

pub(crate) fn random_id() -> Result<String, AppError> {
    let mut buf = [0u8; 16];
    // TODO: Remove expect and return error
    getrandom::fill(&mut buf).map_err(|e| AppError::Internal(Box::new(e)))?; // system RNG unavailable
    Ok(hex::encode(buf))
}

/// Tokens are high-entropy, so a plain SHA-256 is sufficient (see schema notes).
/// Only the hash is ever stored.
pub(crate) fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

// ---------------------------------------------------------------- passwords

pub(crate) async fn hash_password(password: SecretString) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.expose_secret().as_bytes())
            .map(|h| h.to_string())
            .map_err(|e| AppError::Internal(Box::new(e)))
    })
    .await
    .map_err(|e| AppError::Internal(Box::new(e)))?
}

pub(crate) async fn verify_password(password: SecretString, phc_hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&phc_hash).is_ok_and(|parsed| {
            Argon2::default()
                .verify_password(password.expose_secret().as_bytes(), &parsed)
                .is_ok()
        })
    })
    .await
    .unwrap_or(false)
}
