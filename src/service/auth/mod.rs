mod account;
mod attempt;
mod authenticate;
mod login;
mod mail;
mod password;

pub(crate) use account::{create_account, verify_email};
pub(crate) use login::{login, logout};
pub(crate) use password::{forgot_password, reset_password};

pub(crate) async fn init() {
    let _ = login::dummy_hash().await;
}
