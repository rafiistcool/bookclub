//! The admin notification job (#54, D1; `jobs::lock_ids::ADMIN_NOTIFY`): mails every instance
//! admin with a verified address, in their locale, when new items wait in a queue. Reports are
//! mailed at most once an hour, catalog requests as a digest at most once a day. A mail names
//! counts and the oldest item's age only, never a reporter or what they wrote.
//!
//! The state lives on the items (migration 2100): `reports.notified_at` and
//! `catalog_edit_requests.notified_at` say when a mail first counted the item, and the newest of
//! them is the queue's last mail. A run mails a queue when open items without `notified_at`
//! exist and the queue's interval has passed since its last mail; then it stamps those items.
//! An item resolved before the run is not news, and an item that commits after a newer one was
//! mailed is still news. The job runs every few minutes, so the first report after a quiet hour
//! is mailed within minutes.

use std::time::Duration;

use sqlx::PgConnection;

use crate::AppState;
use crate::auth::mail::site;
use crate::email::Email;
use crate::email::templates::{self, QueueNews, Recipient};

/// How often the job looks (`jobs::spawn_all`).
pub const RUN_EVERY: Duration = Duration::from_secs(5 * 60);
/// The least time between two report mails.
pub const REPORTS_EVERY_HOURS: i32 = 1;
/// The least time between two catalog digests.
pub const DIGEST_EVERY_HOURS: i32 = 24;
/// The longest the job waits for one mail.
const SEND_TIMEOUT: Duration = Duration::from_secs(15);

/// One of the two queues the job watches: `reports` or `catalog_edit_requests`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Queue {
    Reports,
    EditRequests,
}

impl Queue {
    fn every_hours(self) -> i32 {
        match self {
            Queue::Reports => REPORTS_EVERY_HOURS,
            Queue::EditRequests => DIGEST_EVERY_HOURS,
        }
    }
}

/// What a run sent: the mails per queue (one per recipient).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NotifyRun {
    pub report_mails: usize,
    pub digest_mails: usize,
}

/// An instance admin who can be mailed.
struct Admin {
    username: String,
    email: String,
    locale: String,
}

/// One run of the job: mails each queue that has news and whose interval has passed.
pub async fn run(state: &AppState, conn: &mut PgConnection) -> anyhow::Result<NotifyRun> {
    let admins = sqlx::query_as!(
        Admin,
        r#"SELECT username, email AS "email!", locale FROM users
           WHERE is_admin AND email_verified_at IS NOT NULL
           ORDER BY id"#
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(NotifyRun {
        report_mails: notify_queue(state, conn, Queue::Reports, &admins).await?,
        digest_mails: notify_queue(state, conn, Queue::EditRequests, &admins).await?,
    })
}

/// Mails `queue`'s news to `admins` when there is news and its interval has passed; returns
/// the mails sent. The news is stamped when at least one mail went out, so a failed send is
/// tried again on the next run, and nothing is lost while no admin has a verified address.
async fn notify_queue(
    state: &AppState,
    conn: &mut PgConnection,
    queue: Queue,
    admins: &[Admin],
) -> anyhow::Result<usize> {
    if admins.is_empty() {
        return Ok(0);
    }
    let Some((news, fresh)) = news(conn, queue).await? else {
        return Ok(0);
    };
    let site = site(&state.config);
    let mut sent = 0;
    for admin in admins {
        let to = Recipient {
            to: &admin.email,
            username: &admin.username,
            locale: &admin.locale,
        };
        let mail = match queue {
            Queue::Reports => templates::admin_reports(site, to, news),
            Queue::EditRequests => templates::admin_requests_digest(site, to, news),
        };
        if send(state, mail).await {
            sent += 1;
        }
    }
    if sent > 0 {
        stamp(conn, queue, &fresh).await?;
    }
    Ok(sent)
}

/// `queue`'s news when its interval has passed since its last mail (the newest `notified_at`):
/// the open items no mail has counted yet, with the open total and the oldest open item's age,
/// and those items' ids. `None` when the interval has not passed or nothing new is open.
async fn news(
    conn: &mut PgConnection,
    queue: Queue,
) -> sqlx::Result<Option<(QueueNews, Vec<i64>)>> {
    let hours = queue.every_hours();
    let row = match queue {
        Queue::Reports => sqlx::query!(
            r#"SELECT count(*) FILTER (WHERE notified_at IS NULL) AS "new!",
                      count(*) AS "open!",
                      array_agg(id ORDER BY id) FILTER (WHERE notified_at IS NULL) AS fresh,
                      floor(extract(epoch FROM now() - min(created_at)) / 3600)::int8
                          AS oldest_hours,
                      NOT EXISTS (SELECT 1 FROM reports
                                  WHERE notified_at > now() - make_interval(hours => $1))
                          AS "due!"
               FROM reports WHERE status = 'open'"#,
            hours
        )
        .fetch_one(&mut *conn)
        .await
        .map(|row| (row.new, row.open, row.fresh, row.oldest_hours, row.due))?,
        Queue::EditRequests => sqlx::query!(
            r#"SELECT count(*) FILTER (WHERE notified_at IS NULL) AS "new!",
                      count(*) AS "open!",
                      array_agg(id ORDER BY id) FILTER (WHERE notified_at IS NULL) AS fresh,
                      floor(extract(epoch FROM now() - min(created_at)) / 3600)::int8
                          AS oldest_hours,
                      NOT EXISTS (SELECT 1 FROM catalog_edit_requests
                                  WHERE notified_at > now() - make_interval(hours => $1))
                          AS "due!"
               FROM catalog_edit_requests WHERE status = 'open'"#,
            hours
        )
        .fetch_one(&mut *conn)
        .await
        .map(|row| (row.new, row.open, row.fresh, row.oldest_hours, row.due))?,
    };
    let (new, open, fresh, oldest_hours, due) = row;
    let fresh = fresh.unwrap_or_default();
    if !due || new == 0 || fresh.is_empty() {
        return Ok(None);
    }
    let news = QueueNews {
        new,
        open,
        oldest_hours: oldest_hours.unwrap_or(0).max(0),
    };
    Ok(Some((news, fresh)))
}

/// Marks the items a mail counted (`ids`, from [`news`]) as mailed now. An item that was
/// resolved meanwhile is stamped too: it was counted.
async fn stamp(conn: &mut PgConnection, queue: Queue, ids: &[i64]) -> sqlx::Result<()> {
    match queue {
        Queue::Reports => sqlx::query!(
            "UPDATE reports SET notified_at = now() WHERE id = ANY($1) AND notified_at IS NULL",
            ids
        )
        .execute(&mut *conn)
        .await
        .map(drop),
        Queue::EditRequests => sqlx::query!(
            "UPDATE catalog_edit_requests SET notified_at = now()
             WHERE id = ANY($1) AND notified_at IS NULL",
            ids
        )
        .execute(&mut *conn)
        .await
        .map(drop),
    }
}

/// Sends one mail; a failure or a send slower than [`SEND_TIMEOUT`] is logged.
async fn send(state: &AppState, mail: Email) -> bool {
    match tokio::time::timeout(SEND_TIMEOUT, state.email.send(mail)).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::warn!(error = %format!("{error:#}"), "admin notification: sending failed");
            false
        }
        Err(_) => {
            tracing::warn!("admin notification: sending timed out");
            false
        }
    }
}
