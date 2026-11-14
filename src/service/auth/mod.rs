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

/// Why a login failed, stored in `login_attempts.failure_reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoginFailure {
    UnknownUser,
    Locked,
    BadPassword,
    Inactive,
}

impl LoginFailure {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownUser => "unknown_user",
            Self::Locked => "locked",
            Self::BadPassword => "bad_password",
            Self::Inactive => "inactive",
        }
    }
}

/// Build the decoy hash eagerly so the first unknown-user login does not pay
/// for both hashing and verification.
pub(crate) async fn warm_decoy_hash() -> anyhow::Result<()> {
    login::decoy_hash()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}
