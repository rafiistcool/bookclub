//! The admin notification job (#54, D1; `jobs::lock_ids::ADMIN_NOTIFY`): mails every instance
//! admin with a verified address, in their locale, when new items wait in a queue. Reports are
//! mailed at most once an hour, catalog requests as a digest at most once a day. A mail names
//! counts and the oldest item's age only, never a reporter or what they wrote.
//!
//! `admin_notify_state` (migration 2100) keeps, per queue, the newest item the last mail counted
//! (`last_item_id`) and when it went out (`sent_at`). A run mails a queue when open items with a
//! larger id exist and the queue's interval has passed since `sent_at`; then both move on. An
//! item resolved before the run is not news. The job runs every few minutes, so the first report
//! after a quiet hour is mailed within minutes.

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

/// One of the two queues the job watches (`admin_notify_state.queue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Queue {
    Reports,
    EditRequests,
}

impl Queue {
    fn key(self) -> &'static str {
        match self {
            Queue::Reports => "reports",
            Queue::EditRequests => "edit_requests",
        }
    }

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
/// the mails sent. The state moves on when at least one mail went out, so a failed send is
/// tried again on the next run, and nothing is lost while no admin has a verified address.
async fn notify_queue(
    state: &AppState,
    conn: &mut PgConnection,
    queue: Queue,
    admins: &[Admin],
) -> anyhow::Result<usize> {
    let due = sqlx::query!(
        r#"SELECT last_item_id,
                  sent_at IS NULL OR sent_at <= now() - make_interval(hours => $2) AS "due!"
           FROM admin_notify_state WHERE queue = $1"#,
        queue.key(),
        queue.every_hours()
    )
    .fetch_one(&mut *conn)
    .await?;
    if !due.due || admins.is_empty() {
        return Ok(0);
    }
    let Some((news, newest)) = news(conn, queue, due.last_item_id).await? else {
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
        sqlx::query!(
            "UPDATE admin_notify_state SET last_item_id = $2, sent_at = now() WHERE queue = $1",
            queue.key(),
            newest
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(sent)
}

/// The open items of `queue` newer than `after` (by id), with the open total and the oldest
/// open item's age, and the newest id; `None` when nothing new is open.
async fn news(
    conn: &mut PgConnection,
    queue: Queue,
    after: i64,
) -> sqlx::Result<Option<(QueueNews, i64)>> {
    let row = match queue {
        Queue::Reports => sqlx::query!(
            r#"SELECT count(*) FILTER (WHERE id > $1) AS "new!",
                          count(*) AS "open!",
                          max(id) FILTER (WHERE id > $1) AS newest,
                          floor(extract(epoch FROM now() - min(created_at)) / 3600)::int8
                              AS oldest_hours
                   FROM reports WHERE status = 'open'"#,
            after
        )
        .fetch_one(&mut *conn)
        .await
        .map(|row| (row.new, row.open, row.newest, row.oldest_hours))?,
        Queue::EditRequests => sqlx::query!(
            r#"SELECT count(*) FILTER (WHERE id > $1) AS "new!",
                          count(*) AS "open!",
                          max(id) FILTER (WHERE id > $1) AS newest,
                          floor(extract(epoch FROM now() - min(created_at)) / 3600)::int8
                              AS oldest_hours
                   FROM catalog_edit_requests WHERE status = 'open'"#,
            after
        )
        .fetch_one(&mut *conn)
        .await
        .map(|row| (row.new, row.open, row.newest, row.oldest_hours))?,
    };
    let (new, open, newest, oldest_hours) = row;
    Ok(newest.filter(|_| new > 0).map(|newest| {
        let news = QueueNews {
            new,
            open,
            oldest_hours: oldest_hours.unwrap_or(0).max(0),
        };
        (news, newest)
    }))
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
