pub(crate) use log::LogMailer;
pub(crate) use smtp::SmtpMailer;

use crate::config::MailSettings;

pub(crate) trait MailerTrait: Send + Sync + 'static {
    async fn send(&self, message: Message) -> Result<(), MailError>;
}

/// A ready-to-send e-mail. Plain text only; add `html: Option<String>` when
/// you have templates.
#[derive(Debug, Clone)]
pub(crate) struct Message {
    pub to: String,
    pub subject: String,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum MailError {
    #[error("invalid address: {0}")]
    Address(String),
    #[error("failed to build message: {0}")]
    Build(String),
    #[error("transport error: {0}")]
    Transport(String),
}

#[derive(Debug)]
pub(crate) enum Mailer {
    Log(LogMailer),
    Smtp(Box<SmtpMailer>),
}

impl Mailer {
    /// Production mailer. Missing SMTP configuration is a startup error in
    /// release builds; development logs messages instead.
    pub(crate) fn from_settings(mail: &MailSettings) -> anyhow::Result<Self> {
        match (&mail.from, &mail.smtp) {
            (Some(from), Some(smtp)) => Ok(Self::Smtp(Box::new(SmtpMailer::new(from, smtp)?))),
            (None, Some(_)) => anyhow::bail!("mail.from must be configured when mail.smtp is set"),
            (_, None) if cfg!(debug_assertions) => Ok(Self::Log(LogMailer)),
            (_, None) => anyhow::bail!("mail.smtp must be configured in release builds"),
        }
    }
}

impl MailerTrait for Mailer {
    async fn send(&self, message: Message) -> Result<(), MailError> {
        match self {
            Self::Log(log_mailer) => log_mailer.send(message).await,
            Self::Smtp(smtp_mailer) => smtp_mailer.send(message).await,
        }
    }
}

mod log {
    use super::{MailError, MailerTrait, Message};

    /// Prints mail to the log instead of sending it. Development only: bodies
    /// contain live tokens.
    #[derive(Debug)]
    pub(crate) struct LogMailer;

    impl MailerTrait for LogMailer {
        #[expect(clippy::unused_async_trait_impl, reason = "keeps the trait uniform")]
        async fn send(&self, m: Message) -> Result<(), MailError> {
            tracing::info!(
                "[dev e-mail] to={} subject={:?}\n{}",
                m.to,
                m.subject,
                m.text
            );
            Ok(())
        }
    }
}

mod smtp {
    use std::time::Duration;

    use lettre::{
        AsyncSmtpTransport, AsyncTransport as _, Message as LettreMessage, Tokio1Executor,
        message::Mailbox, transport::smtp::authentication::Credentials,
    };
    use secrecy::ExposeSecret as _;

    use crate::config::SmtpSettings;

    use super::{MailError, MailerTrait, Message};

    #[derive(Debug)]
    pub(crate) struct SmtpMailer {
        transport: AsyncSmtpTransport<Tokio1Executor>,
        from: Mailbox,
    }

    impl SmtpMailer {
        pub(crate) fn new(from: &str, smtp: &SmtpSettings) -> Result<Self, MailError> {
            let from: Mailbox = from
                .parse()
                .map_err(|_| MailError::Address(from.to_owned()))?;

            let builder = if smtp.starttls {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)
            } else {
                AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host)
            }
            .map_err(|e| MailError::Transport(e.to_string()))?;

            let transport = builder
                .port(smtp.port)
                .credentials(Credentials::new(
                    smtp.username.clone(),
                    smtp.password.expose_secret().to_owned(),
                ))
                .timeout(Some(Duration::from_secs(10)))
                .build();

            Ok(Self { transport, from })
        }
    }

    impl MailerTrait for SmtpMailer {
        async fn send(&self, m: Message) -> Result<(), MailError> {
            let to: Mailbox = m.to.parse().map_err(|_| MailError::Address(m.to.clone()))?;
            let email = LettreMessage::builder()
                .from(self.from.clone())
                .to(to)
                .subject(m.subject)
                .body(m.text)
                .map_err(|e| MailError::Build(e.to_string()))?;

            self.transport
                .send(email)
                .await
                .map(|_| ())
                .map_err(|e| MailError::Transport(e.to_string()))
        }
    }
}
