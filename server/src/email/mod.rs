//! Outgoing mail (docs/phase1/conventions.md §9). Mail is sent after the transaction commits; a
//! send failure is logged and never fails the request.

pub mod templates;

use anyhow::Context;
use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::Config;

/// One plain-text message.
#[derive(Clone, Debug)]
pub struct Email {
    /// A plain address (`auth::rules::valid_email`), never a mailbox with a display name.
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
        // A bare address: a display name or angle brackets in `to` could never pick the
        // recipient.
        let to: Address = msg.to.parse().context("invalid recipient address")?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(Mailbox::new(None, to))
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

    /// The recipient is parsed as a bare address before anything is sent, so a mailbox
    /// spelling cannot choose where the mail goes.
    #[tokio::test]
    async fn smtp_takes_only_bare_recipient_addresses() {
        let smtp = SmtpEmail::new(&config(&[
            ("SMTP_URL", "smtp://127.0.0.1:1"),
            ("EMAIL_FROM", "Bookclub <noreply@example.com>"),
        ]))
        .unwrap();
        for to in ["x <victim@example.test>", "<victim@example.test>"] {
            let email = Email {
                to: to.into(),
                subject: "Hi".into(),
                text: "Body".into(),
            };
            let error = smtp.send(email).await.unwrap_err();
            assert!(
                error.to_string().contains("invalid recipient address"),
                "{to}: {error:#}"
            );
        }
    }
}
