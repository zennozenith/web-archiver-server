use secrecy::ExposeSecret as _;
use tracing::Instrument as _;

use crate::{
    AppState,
    identity::{ResetToken, VerifyToken},
    infra::{MailTransport as _, mailer::OutgoingMail},
    telemetry::redact::MaskedEmail,
};

pub(super) fn verification(public_url: &str, to: &str, token: &VerifyToken) -> OutgoingMail {
    OutgoingMail {
        to: to.to_owned(),
        subject: "Verify your e-mail address".into(),
        text: format!(
            "Welcome!\n\nConfirm your e-mail address by opening this link (valid for 24 hours):\n\n\
             {public_url}/verify?token={}\n\n\
             If you didn't create an account, you can ignore this message.\n",
            token.expose_secret()
        ),
    }
}

/// Sent when someone registers with an address that already has an account.
/// This is the only signal the real owner gets, since the registration form
/// answers identically in both cases.
pub(super) fn already_registered(public_url: &str, to: &str) -> OutgoingMail {
    OutgoingMail {
        to: to.to_owned(),
        subject: "You already have an account".into(),
        text: format!(
            "Someone tried to create an account with this e-mail address, but one already exists.\n\n\
             If it was you, sign in at {public_url}/login, or reset your password at \
             {public_url}/forgot.\n\n\
             If it wasn't you, you can ignore this message.\n"
        ),
    }
}

pub(super) fn password_reset(public_url: &str, to: &str, token: &ResetToken) -> OutgoingMail {
    OutgoingMail {
        to: to.to_owned(),
        subject: "Reset your password".into(),
        text: format!(
            "Someone asked to reset the password for this account.\n\n\
             Open this link to choose a new one (valid for 1 hour):\n\n\
             {public_url}/reset?token={}\n\n\
             If this wasn't you, you can ignore this message. Your password has not changed.\n",
            token.expose_secret()
        ),
    }
}

/// Sends in the background. A mail failure is logged, never returned: the
/// token is already saved, and a visible failure would also reveal whether
/// an address has an account.
pub(super) fn dispatch(state: &AppState, message: OutgoingMail) {
    let mailer = std::sync::Arc::<crate::infra::mailer::Mailer>::clone(&state.mailer);
    // `tokio::spawn` starts a fresh task with no span, so carry the request's
    // context over explicitly; otherwise a send failure would lose its request_id.
    let span = tracing::info_span!("mail.send", to = %MaskedEmail(&message.to));
    tokio::spawn(
        async move {
            match mailer.send(message).await {
                Ok(()) => tracing::debug!("e-mail sent"),
                Err(error) => tracing::error!(%error, "failed to send e-mail"),
            }
        }
        .instrument(span),
    );
}
