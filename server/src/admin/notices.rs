//! Decision notices (#54, Q12 option A): when an admin decides reports, the app mails the
//! decision to each reporter (DSA Art. 16 (5)) and, when something was removed, a statement of
//! reasons to whoever uploaded or added it (Art. 17): the uploader of a cover, the owner of a
//! profile picture, the reader who added a work or edition. "dismiss" mails only the reporters.
//!
//! The mails are built inside the decision's transaction, which knows who reported what, and
//! sent after it commits ([`Notices::send`]); a rollback (a 409, an error) drops them unsent.
//! The answer to the admin comes after the mails were handed to the transport.
//! Only verified addresses get mail, in the recipient's `users.locale`. Neither side is named
//! to the other, and the report's own words (`details`) go to nobody. A notice an admin
//! recorded from an email (`via = 'notice'`) has no reporter account; the admin answers that
//! sender by mail.

use std::time::Duration;

use sqlx::PgConnection;

use crate::AppState;
use crate::auth::mail::site;
use crate::config::Config;
use crate::email::Email;
use crate::email::templates::{self, Outcome, Recipient, Reported};

/// The longest one notice waits for the mail transport.
const SEND_TIMEOUT: Duration = Duration::from_secs(15);

/// What a decision removed, for the statement of reasons.
#[derive(Debug, Clone, Copy)]
pub struct Removal<'a> {
    /// Who uploaded or added it (`media.uploaded_by`, `works.created_by`, …), if anyone.
    pub owner: Option<i64>,
    /// `reports.reason` of the report the admin decided (or of the recorded notice); `None`
    /// when an admin removed it without one.
    pub reason: Option<&'a str>,
}

/// The mails of one decision, sent after its transaction commits.
#[derive(Debug, Default)]
#[must_use = "a decision's notices are sent with `Notices::send` after the commit"]
pub struct Notices(Vec<Email>);

/// A recipient's address and language.
struct Account {
    id: i64,
    username: String,
    email: String,
    locale: String,
    /// The status of the reporter's report (`removed` or `dismissed`); empty for an owner.
    status: String,
}

impl Account {
    fn recipient(&self) -> Recipient<'_> {
        Recipient {
            to: &self.email,
            username: &self.username,
            locale: &self.locale,
        }
    }
}

/// The notices of the reports `admin_id` resolved in this transaction (`resolved_at = now()`,
/// so the caller runs this after its last update of `reports`), all about `what`, plus the
/// statement of reasons of `removal` when the decision removed something. A reporter who is
/// also the owner gets the statement only. With `gone` (what was reported no longer shows as
/// reported: a profile picture its owner replaced or deleted, a work merged away) the reporters
/// learn that instead of a decision on it.
pub async fn decided(
    conn: &mut PgConnection,
    config: &Config,
    admin_id: i64,
    what: Reported<'_>,
    removal: Option<Removal<'_>>,
    gone: bool,
) -> sqlx::Result<Notices> {
    let site = site(config);
    let contact = config.contact_email.as_deref();
    let owner = match removal.and_then(|removal| removal.owner) {
        Some(owner) => {
            sqlx::query_as!(
                Account,
                r#"SELECT id, username, email AS "email!", locale, '' AS "status!"
               FROM users WHERE id = $1 AND email_verified_at IS NOT NULL"#,
                owner
            )
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    let reporters = sqlx::query_as!(
        Account,
        r#"SELECT DISTINCT ON (u.id) u.id, u.username, u.email AS "email!", u.locale,
                  r.status AS "status!"
           FROM reports r
           JOIN users u ON u.id = r.created_by
           WHERE r.resolved_by = $1 AND r.resolved_at = now()
             AND u.email_verified_at IS NOT NULL
           ORDER BY u.id, r.id"#,
        admin_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut mails = Vec::new();
    for reporter in &reporters {
        if owner.as_ref().is_some_and(|owner| owner.id == reporter.id) {
            continue;
        }
        let outcome = if gone {
            Outcome::Gone
        } else if reporter.status == "removed" {
            Outcome::Removed
        } else {
            Outcome::Kept
        };
        mails.push(templates::report_decision(
            site,
            reporter.recipient(),
            what,
            outcome,
            contact,
        ));
    }
    if let (Some(owner), Some(removal)) = (&owner, removal) {
        mails.push(templates::statement_of_reasons(
            site,
            owner.recipient(),
            what,
            removal.reason,
            contact,
        ));
    }
    Ok(Notices(mails))
}

impl Notices {
    /// The mails, oldest first (for tests and logs).
    pub fn mails(&self) -> &[Email] {
        &self.0
    }

    /// Sends the mails after the decision committed, one after the other; a failure or a send
    /// slower than 15 seconds is logged and never fails the decision. They go out in a task of
    /// their own that the answer waits for: the admin's answer comes once they were handed to
    /// the transport, and an admin who closes the page meanwhile does not cancel them.
    pub async fn send(self, state: &AppState) {
        if self.0.is_empty() {
            return;
        }
        let state = state.clone();
        let sending = tokio::spawn(async move {
            for mail in self.0 {
                match tokio::time::timeout(SEND_TIMEOUT, state.email.send(mail)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(error = %format!("{error:#}"), "decision notice: sending failed");
                    }
                    Err(_) => tracing::warn!("decision notice: sending timed out"),
                }
            }
        });
        if let Err(error) = sending.await {
            tracing::warn!(%error, "decision notice: the sending task failed");
        }
    }
}
