pub(crate) use log::LogMailer;
#[cfg(test)]
pub(crate) use memory::MemoryMailer;
// pub(crate) use smtp::SmtpMailer;

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
    // Smtp(SmtpMailer),
    #[cfg(test)]
    Memory(MemoryMailer),
}

mod log {
    use super::{MailError, Message};

    /// Prints mail to the log instead of sending it. Development only: bodies
    /// contain live tokens.
    #[derive(Debug)]
    pub(crate) struct LogMailer;

    impl LogMailer {
        pub(crate) async fn send(&self, m: Message) -> Result<(), MailError> {
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
    // use lettre::{
    //     AsyncSmtpTransport, AsyncTransport, Message as LettreMessage, Tokio1Executor, message::Mailbox,
    //     transport::smtp::authentication::Credentials,
    // };

    // use super::{MailError, Mailer, Message};
    // pub(crate) struct SmtpMailer {
    //     transport: AsyncSmtpTransport<Tokio1Executor>,
    //     from: Mailbox,
    // }

    // impl SmtpMailer {
    //     pub(crate) fn new(from: &str, smtp: &SmtpSettings) -> Result<Self, MailError> {
    //         let from: Mailbox = from
    //             .parse()
    //             .map_err(|_| MailError::Address(from.to_owned()))?;

    //         let builder = if smtp.starttls {
    //             AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)
    //         } else {
    //             AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host) // implicit TLS (465)
    //         }
    //         .map_err(|e| MailError::Transport(e.to_string()))?;

    //         let transport = builder
    //             .port(smtp.port)
    //             .credentials(Credentials::new(
    //                 smtp.username.clone(),
    //                 smtp.password.expose_secret().to_owned(),
    //             ))
    //             .timeout(Some(std::time::Duration::from_secs(10)))
    //             .build();

    //         Ok(Self { transport, from })
    //     }
    // }

    // impl Mailer for SmtpMailer {
    //     async fn send(&self, m: Message) -> Result<(), MailError> {
    //         let to: Mailbox = m.to.parse().map_err(|_| MailError::Address(m.to.clone()))?;
    //         let email = LettreMessage::builder()
    //             .from(self.from.clone())
    //             .to(to)
    //             .subject(m.subject)
    //             .body(m.text)
    //             .map_err(|e| MailError::Build(e.to_string()))?;

    //         self.transport
    //             .send(email)
    //             .await
    //             .map(|_| ())
    //             .map_err(|e| MailError::Transport(e.to_string()))
    //     }
    // }
}

#[cfg(test)]
mod memory {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use super::{MailError, Message};

    #[derive(Debug, Clone, Default)]
    pub(crate) struct MemoryMailer(pub Arc<Mutex<Vec<Message>>>);

    impl MemoryMailer {
        pub(crate) async fn sent(&self) -> Vec<Message> {
            self.0.lock().await.clone()
        }

        pub(crate) async fn send(&self, m: Message) -> Result<(), MailError> {
            self.0.lock().await.push(m);
            Ok(())
        }
    }
}
