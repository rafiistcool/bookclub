//! Outgoing mail (docs/phase1/conventions.md §9). Mail is sent after the transaction commits; a
//! send failure is logged and never fails the request.

pub mod templates;

use anyhow::Context;
use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::auth::random;
use crate::config::Config;

/// One plain-text message.
#[derive(Clone, Debug)]
pub struct Email {
    /// A plain address (`auth::rules::valid_email`), never a mailbox with a display name.
    pub to: String,
    pub subject: String,
    pub text: String,
}

/// Sends mail. Production uses [`SmtpEmail`], or [`NoEmail`] while no mail provider exists;
/// development [`LogEmail`], tests `CapturingEmail` (tests/common).
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

/// What [`NoEmail`] fails with, and what `send-test-mail` prints under `EMAIL_TRANSPORT=none`.
pub const MAIL_OFF: &str = "mail is off (EMAIL_TRANSPORT=none)";

/// The transport while no mail provider exists (`EMAIL_TRANSPORT=none`, the private move #52):
/// every send fails with [`MAIL_OFF`], so callers treat the mail as not sent (a reset or
/// verification link is voided, an admin notice stamps nothing). It logs nothing itself: no
/// recipient, subject, body or link, so a reset link never reaches the log outside DEV_MODE
/// (docs/phase1/conventions.md §11, §19). `auth::mail::deliver` logs that a mail was not sent.
pub struct NoEmail;

#[async_trait::async_trait]
impl EmailSender for NoEmail {
    async fn send(&self, _msg: Email) -> anyhow::Result<()> {
        Err(anyhow::Error::msg(MAIL_OFF))
    }
}

/// Whether `error` is [`NoEmail`]'s refusal rather than a failed send.
pub fn is_mail_off(error: &anyhow::Error) -> bool {
    error.downcast_ref::<&str>() == Some(&MAIL_OFF)
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

    /// The message for `msg`, with an explicit `Message-ID` (see [`message_id`]). lettre adds
    /// the `Date` header itself.
    fn message(&self, msg: Email) -> anyhow::Result<Message> {
        // A bare address: a display name or angle brackets in `to` could never pick the
        // recipient.
        let to: Address = msg.to.parse().context("invalid recipient address")?;
        Message::builder()
            .from(self.from.clone())
            .to(Mailbox::new(None, to))
            .message_id(Some(message_id(self.from.email.domain())))
            .subject(msg.subject)
            .header(ContentType::TEXT_PLAIN)
            .body(msg.text)
            .context("cannot build the email")
    }
}

/// A `Message-ID` of `<uuid@domain>`, with the domain of `EMAIL_FROM` and a random version 4
/// UUID. lettre adds no `Message-ID` by itself, and its own default names the host, which in a
/// container is the container id; spam filters count both against a message.
fn message_id(domain: &str) -> String {
    let mut bytes = [0u8; 16];
    random::fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // the RFC 9562 variant
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "<{}-{}-{}-{}-{}@{domain}>",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The message `bookclub-server send-test-mail` sends: English, with no link that does
/// anything. It checks `SMTP_URL`, `EMAIL_FROM` and the sender domain's SPF, DKIM and DMARC.
pub fn test_mail(config: &Config, to: &str) -> Email {
    Email {
        to: to.to_owned(),
        subject: format!("{} test mail", config.service_name),
        text: format!(
            "This is a test mail from {service} at {url}.\n\n\
             An operator sent it with `bookclub-server send-test-mail` to check the outgoing \
             mail settings. Nothing needs to be done.\n",
            service = config.service_name,
            url = config.public_url,
        ),
    }
}

#[async_trait::async_trait]
impl EmailSender for SmtpEmail {
    async fn send(&self, msg: Email) -> anyhow::Result<()> {
        let message = self.message(msg)?;
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

    /// `EMAIL_TRANSPORT=none`: every send fails with `MAIL_OFF`, which `is_mail_off` tells
    /// apart from a failed send.
    #[tokio::test]
    async fn the_none_transport_sends_nothing() {
        let email = Email {
            to: "ada@example.test".into(),
            subject: "Hi".into(),
            text: "Body".into(),
        };
        let error = NoEmail.send(email).await.unwrap_err();
        assert_eq!(error.to_string(), "mail is off (EMAIL_TRANSPORT=none)");
        assert!(is_mail_off(&error));
        assert!(!is_mail_off(&anyhow::anyhow!("SMTP send failed")));
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

    /// #51: every SMTP message carries a `Message-ID` on the sender's domain (not the
    /// container's hostname) and a `Date`. (The SMTP pool needs a runtime.)
    #[tokio::test]
    async fn smtp_messages_carry_a_message_id_on_the_sender_domain() {
        let smtp = SmtpEmail::new(&config(&[
            ("SMTP_URL", "smtp://127.0.0.1:1"),
            ("EMAIL_FROM", "Bookclub <noreply@mail.example.com>"),
        ]))
        .unwrap();
        let shape = regex::Regex::new(
            r"^<[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}@mail\.example\.com>$",
        )
        .unwrap();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let message = smtp
                .message(Email {
                    to: "ada@example.test".into(),
                    subject: "Hi".into(),
                    text: "Body".into(),
                })
                .unwrap();
            let headers = message.headers();
            let id = headers.get_raw("Message-ID").unwrap().to_owned();
            assert!(shape.is_match(&id), "{id}");
            assert!(headers.get_raw("Date").is_some());
            assert_eq!(
                headers.get_raw("From"),
                Some("Bookclub <noreply@mail.example.com>")
            );
            ids.push(id);
        }
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn the_test_mail_names_the_service_and_its_origin() {
        let config = config(&[
            ("SERVICE_NAME", "Shelfmates"),
            ("PUBLIC_URL", "https://books.example.com"),
        ]);
        let mail = test_mail(&config, "ada@example.test");
        assert_eq!(mail.to, "ada@example.test");
        assert_eq!(mail.subject, "Shelfmates test mail");
        assert!(
            mail.text.starts_with(
                "This is a test mail from Shelfmates at https://books.example.com.\n\n"
            ),
            "{}",
            mail.text
        );
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
