use secrecy::ExposeSecret as _;

use crate::{
    AppState,
    auth::{ResetToken, VerifyToken},
    infra::mailer::Message,
};

pub(super) fn verification(public_url: &str, to: &str, token: &VerifyToken) -> Message {
    Message {
        to: to.to_owned(),
        subject: "Verify your e-mail address".into(),
        text: format!(
            "Welcome!\n\nConfirm your e-mail address by opening this link (valid for 24 hours):\n\n\
             {public_url}/verify-email?token={}\n\n\
             If you didn't create an account, you can ignore this message.\n",
            token.expose_secret()
        ),
    }
}

pub(super) fn password_reset(public_url: &str, to: &str, token: &ResetToken) -> Message {
    Message {
        to: to.to_owned(),
        subject: "Reset your password".into(),
        text: format!(
            "Someone asked to reset the password for this account.\n\n\
             Open this link to choose a new one (valid for 1 hour):\n\n\
             {public_url}/reset-password?token={}\n\n\
             If this wasn't you, you can ignore this message. Your password has not changed.\n",
            token.expose_secret()
        ),
    }
}

/// Sends in the background. A mail failure is logged, never returned: the
/// token is already saved, and a visible failure would also reveal whether
/// an address has an account.
pub(super) fn dispatch(ctx: &AppState, message: Message) {
    let mailer = ctx.mailer.clone();
    tokio::spawn(async move {
        let to = message.to.clone();
        if let Err(e) = mailer.send(message).await {
            tracing::error!(%to, "failed to send e-mail: {e}");
        }
    });
}
