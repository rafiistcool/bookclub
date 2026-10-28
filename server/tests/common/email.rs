//! [`CapturingEmail`]: the test mail transport.

use std::sync::Mutex;

use bookclub_server::email::{Email, EmailSender};

/// Keeps every message instead of sending it.
#[derive(Debug, Default)]
pub struct CapturingEmail {
    sent: Mutex<Vec<Email>>,
}

impl CapturingEmail {
    /// Every message so far, oldest first.
    pub fn sent(&self) -> Vec<Email> {
        self.sent.lock().unwrap().clone()
    }

    /// The messages to `to` (case-insensitive), oldest first.
    pub fn sent_to(&self, to: &str) -> Vec<Email> {
        self.sent()
            .into_iter()
            .filter(|mail| mail.to.eq_ignore_ascii_case(to))
            .collect()
    }
}

#[async_trait::async_trait]
impl EmailSender for CapturingEmail {
    async fn send(&self, msg: Email) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(msg);
        Ok(())
    }
}
