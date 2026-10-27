//! Outgoing mail (docs/phase1/conventions.md §9). Mail is sent after the transaction commits; a
//! send failure is logged and never fails the request.

pub mod templates;

use anyhow::Context;
use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::Config;

/// One plain-text message.
#[derive(Clone, Debug)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub text: String,
}

/// Sends mail. Production uses [`SmtpEmail`], development [`LogEmail`], tests `CapturingEmail`
/// (tests/common).
#[async_trait::async_trait]
pub trait EmailSender: Send + Sync {
    async fn send(&self, msg: Email) -> anyhow::Result<()>;
}

/// The development transport (`EMAIL_TRANSPORT=log`): logs recipient, subject and the full body
/// at INFO, so links can be copied from the log.
pub struct LogEmail;

#[async_trait::async_trait]
impl EmailSender for LogEmail {
    async fn send(&self, msg: Email) -> anyhow::Result<()> {
        tracing::info!(to = %msg.to, subject = %msg.subject, body = %msg.text, "email (log transport)");
        Ok(())
    }
}

/// SMTP through lettre with rustls and a connection pool, configured by `SMTP_URL` and
/// `EMAIL_FROM`.
pub struct SmtpEmail {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpEmail {
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        let url = config.smtp_url.as_deref().context("SMTP_URL is not set")?;
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            .context(
                "SMTP_URL is not a valid SMTP URL, e.g. smtps://user:pass@smtp.example.eu:465",
            )?
            .build();
        let from = config
            .email_from
            .as_deref()
            .context("EMAIL_FROM is not set")?
            .parse()
            .context("EMAIL_FROM is not a valid mailbox, e.g. Bookclub <noreply@example.com>")?;
        Ok(Self { transport, from })
    }
}

#[async_trait::async_trait]
impl EmailSender for SmtpEmail {
    async fn send(&self, msg: Email) -> anyhow::Result<()> {
        let message = Message::builder()
            .from(self.from.clone())
            .to(msg.to.parse().context("invalid recipient address")?)
            .subject(msg.subject)
            .header(ContentType::TEXT_PLAIN)
            .body(msg.text)
            .context("cannot build the email")?;
        self.transport
            .send(message)
            .await
            .context("SMTP send failed")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(extra: &[(&str, &str)]) -> Config {
        let mut pairs = vec![("DATABASE_URL", "postgres://x"), ("DEV_MODE", "true")];
        pairs.extend_from_slice(extra);
        Config::from_lookup(|key| {
            pairs
                .iter()
                .rev()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        })
        .unwrap()
    }

    #[tokio::test]
    async fn log_transport_accepts_everything() {
        let email = Email {
            to: "ada@example.test".into(),
            subject: "Hi".into(),
            text: "Body".into(),
        };
        LogEmail.send(email).await.unwrap();
    }

    #[tokio::test]
    async fn smtp_needs_a_valid_url_and_sender() {
        let ok = config(&[
            ("SMTP_URL", "smtps://user:pass@smtp.example.eu:465"),
            ("EMAIL_FROM", "Bookclub <noreply@example.com>"),
        ]);
        assert!(SmtpEmail::new(&ok).is_ok());
        let bad_from = config(&[
            ("SMTP_URL", "smtps://user:pass@smtp.example.eu:465"),
            ("EMAIL_FROM", "not a mailbox"),
        ]);
        let error = SmtpEmail::new(&bad_from).err().unwrap();
        assert!(error.to_string().contains("EMAIL_FROM"));
        let error = SmtpEmail::new(&config(&[])).err().unwrap();
        assert!(error.to_string().contains("SMTP_URL"));
    }
}
