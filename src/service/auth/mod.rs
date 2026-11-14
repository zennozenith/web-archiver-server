mod account;
mod attempt;
mod authenticate;
mod login;
mod mail;
mod password;

pub(crate) use account::{create_account, verify_email};
pub(crate) use authenticate::authenticate;
pub(crate) use login::{login, logout};
pub(crate) use password::{forgot_password, reset_password};

/// Build the decoy hash eagerly so the first unknown-user login does not pay
/// for both hashing and verification.
pub(crate) async fn warm_decoy_hash() {
    let _ = login::decoy_hash().await;
}
