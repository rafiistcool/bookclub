//! The admin notification job (#54, D1): `admin::notify::run`, which `jobs::spawn_all` runs
//! every five minutes under `jobs::lock_ids::ADMIN_NOTIFY` (the last test goes through
//! `spawn_all`; the others call the job directly). New reports are mailed at most
//! hourly, new catalog requests as a daily digest, to every instance admin with a verified
//! address, in their locale.

mod common;

use std::sync::Arc;
use std::time::Duration;

use bookclub_server::admin::notify::{self, NotifyRun};
use bookclub_server::catalog::shared::ingest::{CoverTarget, Source, ol_cover_media, set_cover};
use bookclub_server::email::{Email, EmailSender};
use bookclub_server::{AppState, jobs};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

/// A work showing the Open Library cover `ol_cover_id`; returns the picture's public id.
async fn picture(app: &TestApp, title: &str, ol_cover_id: i64) -> String {
    let work = app.work(title).await;
    let mut conn = app.db.acquire().await.unwrap();
    let media = ol_cover_media(&mut conn, ol_cover_id)
        .await
        .unwrap()
        .unwrap();
    let target = CoverTarget::Work(work.id);
    let set = set_cover(&mut conn, target, media, Source::OpenLibrary, false).await;
    assert!(set.unwrap());
    sqlx::query_scalar("SELECT public_id FROM media WHERE id = $1")
        .bind(media)
        .fetch_one(&mut *conn)
        .await
        .unwrap()
}

async fn report(client: &TestClient, public_id: &str) {
    client
        .post(
            &format!("/api/media/{public_id}/reports"),
            json!({"reason": "offensive"}),
        )
        .await
        .assert_status(201);
}

async fn request(client: &TestClient, work: i64, pages: i64) {
    client
        .post(
            &format!("/api/books/works/{work}/edit-requests"),
            json!({"fields": {"pages": pages}}),
        )
        .await
        .assert_status(201);
}

async fn run_with(state: &AppState, db: &PgPool) -> NotifyRun {
    let mut conn = db.acquire().await.unwrap();
    notify::run(state, &mut conn).await.unwrap()
}

async fn run(app: &TestApp) -> NotifyRun {
    run_with(&app.state, &app.db).await
}

/// Moves the last mail of a queue `hours` into the past: every item of `table` (`reports` or
/// `catalog_edit_requests`) that a mail counted gets `notified_at` that long ago.
async fn sent_hours_ago(app: &TestApp, table: &str, hours: i64) {
    app.exec(&format!(
        "UPDATE {table} SET notified_at = now() - interval '{hours} hours'
         WHERE notified_at IS NOT NULL"
    ))
    .await;
}

/// The admins and readers of every test: `root` (English) and `wurzel` (German) are verified
/// instance admins, `ghost` an admin without a verified address, `ada` a reader.
async fn people(app: &TestApp) -> (TestClient, TestClient) {
    app.admin("root").await;
    let wurzel = app.admin("wurzel").await;
    app.exec(&format!(
        "UPDATE users SET locale = 'de' WHERE id = {}",
        wurzel.id()
    ))
    .await;
    let ghost = app.user_unverified("ghost").await;
    app.exec(&format!(
        "UPDATE users SET is_admin = true WHERE id = {}",
        ghost.id()
    ))
    .await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    (ada, grace)
}

fn recipients(mails: &[Email]) -> Vec<&str> {
    let mut to: Vec<&str> = mails.iter().map(|mail| mail.to.as_str()).collect();
    to.sort();
    to
}

/// Acceptance (#54): one mail per verified admin, in their locale, and none to an unverified
/// admin or a reader; a second run with nothing new sends nothing; a new report sends again
/// once the hour since the last mail has passed, and not before.
#[sqlx::test]
async fn new_reports_reach_every_verified_admin_at_most_hourly(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = people(&app).await;
    assert_eq!(run(&app).await, NotifyRun::default(), "nothing waits yet");
    assert!(app.sent_emails().is_empty());

    let first = picture(&app, "Circe", 101).await;
    report(&ada, &first).await;
    report(&grace, &first).await;
    let sent = run(&app).await;
    assert_eq!(
        sent,
        NotifyRun {
            report_mails: 2,
            digest_mails: 0
        }
    );
    let mails = app.sent_emails();
    assert_eq!(
        recipients(&mails),
        ["root@example.test", "wurzel@example.test"]
    );
    let link = format!("{}/admin?tab=reports", app.config().public_url);
    let english = &app.email.sent_to("root@example.test")[0];
    assert_eq!(english.subject, "2 new reports on Bookclub");
    assert!(english.text.starts_with("Hi root,"));
    assert!(english.text.contains(&link), "{}", english.text);
    assert!(english.text.contains("2 reports are open in all"));
    let german = &app.email.sent_to("wurzel@example.test")[0];
    assert_eq!(german.subject, "2 neue Meldungen bei Bookclub");
    assert!(german.text.starts_with("Hallo wurzel,"));
    assert!(german.text.contains(&link));
    // Counts only: no reporter and nothing a reporter wrote.
    for mail in &mails {
        assert!(!mail.text.contains("ada") && !mail.text.contains("grace"));
    }

    assert_eq!(run(&app).await, NotifyRun::default(), "nothing new");
    assert_eq!(app.sent_emails().len(), 2);

    let second = picture(&app, "Ich bin Circe", 102).await;
    report(&ada, &second).await;
    assert_eq!(run(&app).await, NotifyRun::default(), "within the hour");
    sent_hours_ago(&app, "reports", 1).await;
    assert_eq!(run(&app).await.report_mails, 2);
    let again = &app.email.sent_to("root@example.test")[1];
    assert_eq!(again.subject, "1 new report on Bookclub");
    assert!(
        again
            .text
            .contains("there is 1 new report on Bookclub. 3 reports are open")
    );
    assert_eq!(app.sent_emails().len(), 4);
    assert_eq!(run(&app).await, NotifyRun::default());
}

/// A report an admin resolved before the job ran is no news, and an earlier report that is
/// still open is counted as open but not as new.
#[sqlx::test]
async fn only_open_reports_are_news(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = people(&app).await;
    let first = picture(&app, "Circe", 101).await;
    report(&ada, &first).await;
    app.exec("UPDATE reports SET status = 'dismissed', resolved_at = now()")
        .await;
    assert_eq!(run(&app).await, NotifyRun::default());

    let second = picture(&app, "Ich bin Circe", 102).await;
    report(&grace, &second).await;
    assert_eq!(run(&app).await.report_mails, 2);
    let mail = &app.email.sent_to("root@example.test")[0];
    assert_eq!(mail.subject, "1 new report on Bookclub");
}

/// An open report that no mail counted is news even when a report with a larger id was mailed
/// before it: two reports filed at once can commit in either order, so the job goes by what each
/// report records (`notified_at`), not by the largest id it has seen.
#[sqlx::test]
async fn a_report_that_commits_late_is_still_news(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = people(&app).await;
    let first = picture(&app, "Circe", 101).await;
    report(&ada, &first).await;
    let second = picture(&app, "Ich bin Circe", 102).await;
    report(&grace, &second).await;
    // As if a run had seen the second report while the first one's transaction was still open.
    app.exec(
        "UPDATE reports SET notified_at = now() - interval '1 hour'
         WHERE id = (SELECT max(id) FROM reports)",
    )
    .await;
    assert_eq!(run(&app).await.report_mails, 2);
    let mail = &app.email.sent_to("root@example.test")[0];
    assert_eq!(mail.subject, "1 new report on Bookclub");
    assert!(
        mail.text.contains("2 reports are open in all"),
        "{}",
        mail.text
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM reports WHERE notified_at IS NULL")
            .await,
        0
    );
    assert_eq!(run(&app).await, NotifyRun::default(), "nothing new");
}

/// Catalog requests go out as a digest at most once a day, with a link to the requests tab.
#[sqlx::test]
async fn catalog_requests_go_out_as_a_daily_digest(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = people(&app).await;
    let work = app.work("Circe").await;
    request(&ada, work.id, 393).await;
    assert_eq!(
        run(&app).await,
        NotifyRun {
            report_mails: 0,
            digest_mails: 2
        }
    );
    let link = format!("{}/admin?tab=requests", app.config().public_url);
    let english = &app.email.sent_to("root@example.test")[0];
    assert_eq!(english.subject, "1 new catalog request on Bookclub");
    assert!(english.text.contains(&link));
    let german = &app.email.sent_to("wurzel@example.test")[0];
    assert_eq!(german.subject, "1 neue Katalog-Anfrage bei Bookclub");

    request(&grace, work.id, 400).await;
    assert_eq!(run(&app).await, NotifyRun::default(), "within the day");
    sent_hours_ago(&app, "catalog_edit_requests", 23).await;
    assert_eq!(
        run(&app).await,
        NotifyRun::default(),
        "still within the day"
    );
    sent_hours_ago(&app, "catalog_edit_requests", 24).await;
    assert_eq!(run(&app).await.digest_mails, 2);
    let digest = &app.email.sent_to("root@example.test")[1];
    assert_eq!(digest.subject, "1 new catalog request on Bookclub");
    assert!(digest.text.contains("2 requests are open in all"));
    assert!(app.email.sent_to("ghost@example.test").is_empty());
    assert!(app.email.sent_to("ada@example.test").is_empty());
}

/// A transport that refuses every mail.
struct Refusing;

#[async_trait::async_trait]
impl EmailSender for Refusing {
    async fn send(&self, _msg: Email) -> anyhow::Result<()> {
        anyhow::bail!("the mail server is down")
    }
}

/// When no mail goes out, the news stays news: the next run sends it. Without a verified admin
/// nothing is sent and nothing is marked.
#[sqlx::test]
async fn news_waits_until_a_mail_goes_out(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let public_id = picture(&app, "Circe", 101).await;
    report(&ada, &public_id).await;
    assert_eq!(run(&app).await, NotifyRun::default(), "no admin to tell");

    app.admin("root").await;
    let mut down = app.state.clone();
    down.email = Arc::new(Refusing);
    assert_eq!(run_with(&down, &app.db).await, NotifyRun::default());
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM reports WHERE notified_at IS NOT NULL")
            .await,
        0
    );
    assert_eq!(run(&app).await.report_mails, 1);
    assert_eq!(
        app.email.sent_to("root@example.test")[0].subject,
        "1 new report on Bookclub"
    );
}

/// A transport that holds every mail until the test lets it go, and says when one waits.
struct Held {
    waiting: mpsc::UnboundedSender<()>,
    go: Semaphore,
    sent: CapturingEmail,
}

#[async_trait::async_trait]
impl EmailSender for Held {
    async fn send(&self, msg: Email) -> anyhow::Result<()> {
        let _ = self.waiting.send(());
        self.go.acquire().await?.forget();
        self.sent.send(msg).await
    }
}

/// Advisory locks on `key` (the one-key form) held in this test's database.
async fn advisory_locks_held(db: &PgPool, key: i64) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_locks
         WHERE locktype = 'advisory' AND granted AND objsubid = 1
           AND database = (SELECT oid FROM pg_database WHERE datname = current_database())
           AND classid::int8 = $1 >> 32 AND objid::int8 = $1 & 4294967295",
    )
    .bind(key)
    .fetch_one(db)
    .await
    .unwrap()
}

/// The server runs the job: `jobs::spawn_all` runs it once at startup, and while its mail goes
/// out the job's session holds the `ADMIN_NOTIFY` advisory lock (so a second instance skips the
/// run, `jobs::run_locked`). Nothing here calls the job itself.
#[sqlx::test]
async fn the_server_runs_the_job_at_startup_under_its_lock(db: PgPool) {
    let app = TestApp::new(db).await;
    app.admin("root").await;
    let ada = app.user("ada").await;
    let public_id = picture(&app, "Circe", 101).await;
    report(&ada, &public_id).await;

    let (waiting, mut mail_waits) = mpsc::unbounded_channel();
    let held = Arc::new(Held {
        waiting,
        go: Semaphore::new(0),
        sent: CapturingEmail::default(),
    });
    let mut state = app.state.clone();
    state.email = held.clone();
    let shutdown = CancellationToken::new();
    let mut background = jobs::spawn_all(state, shutdown.clone());

    tokio::time::timeout(Duration::from_secs(10), mail_waits.recv())
        .await
        .expect("no admin mail within 10 s of startup")
        .expect("the transport is gone");
    assert_eq!(
        advisory_locks_held(&app.db, jobs::lock_ids::ADMIN_NOTIFY).await,
        1,
        "the mail goes out under ADMIN_NOTIFY"
    );
    held.go.add_permits(1);
    let mut mails = Vec::new();
    for _ in 0..100 {
        mails = held.sent.sent();
        if !mails.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(recipients(&mails), ["root@example.test"]);
    assert_eq!(mails[0].subject, "1 new report on Bookclub");

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(result) = background.join_next().await {
            result.expect("a background job failed");
        }
    })
    .await
    .expect("background jobs did not stop within 5 s");
    assert_eq!(
        advisory_locks_held(&app.db, jobs::lock_ids::ADMIN_NOTIFY).await,
        0,
        "released after the run"
    );
    assert_eq!(held.sent.sent().len(), 1, "one run at startup");
}
