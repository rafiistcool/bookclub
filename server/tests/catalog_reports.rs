//! Reports on works and editions readers added (#54, Q11 option B):
//! `POST /api/books/works/{work_id}/reports`, `POST /api/books/editions/{edition_id}/reports`,
//! and their decisions in the one admin queue, with the decision notices (Q12 option A).

mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// `carl` adds "My Zine" (with an edition: a publisher and pages) through the API; returns the
/// work id and the edition id.
async fn zine(app: &TestApp, carl: &TestClient) -> (i64, i64) {
    let res = carl
        .post(
            "/api/books/custom",
            json!({
                "title": "My Zine",
                "authors": "Carl Rude",
                "description": "Copied from the publisher's site.",
                "publisher": "Basement Press",
                "pages": 32
            }),
        )
        .await;
    res.assert_status(201);
    let work = res.json()["id"].as_i64().unwrap();
    let edition = app
        .scalar_i64(&format!("SELECT id FROM editions WHERE work_id = {work}"))
        .await;
    (work, edition)
}

async fn report_work(who: &TestClient, work: impl std::fmt::Display, body: Value) -> TestResponse {
    who.post(&format!("/api/books/works/{work}/reports"), body)
        .await
}

async fn report_edition(who: &TestClient, edition: i64, body: Value) -> TestResponse {
    who.post(&format!("/api/books/editions/{edition}/reports"), body)
        .await
}

async fn resolve(admin: &TestClient, report: i64, action: &str) -> TestResponse {
    admin
        .post(
            &format!("/api/admin/reports/{report}/resolve"),
            json!({ "action": action }),
        )
        .await
}

fn mails_to(app: &TestApp, username: &str) -> Vec<bookclub_server::email::Email> {
    app.email.sent_to(&format!("{username}@example.test"))
}

/// Acceptance (#54): a report on a user-added work shows up in the queue, naming the book and
/// the reporter; a repeat answers with the same report.
#[sqlx::test]
async fn a_report_on_a_user_added_work_shows_up_in_the_queue(db: PgPool) {
    let app = TestApp::new(db).await;
    let carl = app.user("carl").await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (work, _) = zine(&app, &carl).await;

    let res = report_work(
        &ada,
        work,
        json!({"reason": "copyright", "details": "  The blurb is copied.  "}),
    )
    .await;
    res.assert_status(201);
    assert_eq!(res.json()["status"], "open");
    let id = res.json()["id"].as_i64().unwrap();
    let again = report_work(&ada, work, json!({"reason": "other"})).await;
    again.assert_status(201);
    assert_eq!(again.json()["id"], id);

    let items = root.get("/api/admin/reports").await.json()["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 1);
    let item = &items[0];
    assert_eq!(item["id"], id);
    assert_eq!(item["target_kind"], "work");
    assert_eq!(item["via"], "app");
    assert_eq!(item["reason"], "copyright");
    assert_eq!(item["details"], "The blurb is copied.");
    assert_eq!(item["book"]["id"], work);
    assert_eq!(item["book"]["title"], "My Zine");
    assert_eq!(item["book"]["custom"], true);
    assert_eq!(item["created_by"]["username"], "ada");
    assert_eq!(item["media_public_id"], Value::Null);
    assert_eq!(item["cover_url"], Value::Null);
    assert_eq!(item["avatar_url"], Value::Null);
    assert_eq!(item["target_user"], Value::Null);
    let summary = root.get("/api/admin/summary").await.json();
    assert_eq!(summary["reports"]["open"], 1);
}

/// "remove" withdraws what the reader typed and keeps the book, so shelves keep it: the title
/// becomes "[removed]", subtitle, authors, description and subjects go, with provenance
/// `admin`. Every open report of the work is resolved; the reporters get the decision and the
/// reader who added it a statement of reasons, neither naming the other.
#[sqlx::test]
async fn remove_withdraws_the_text_of_a_work(db: PgPool) {
    let app = TestApp::new(db).await;
    let carl = app.user("carl").await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let root = app.admin("root").await;
    let (work, _) = zine(&app, &carl).await;
    carl.post("/api/shelf", json!({"work_id": work, "status": "finished"}))
        .await;
    let first = report_work(
        &ada,
        work,
        json!({"reason": "offensive", "details": "Ada here"}),
    )
    .await
    .json()["id"]
        .as_i64()
        .unwrap();
    report_work(&bob, work, json!({"reason": "other"}))
        .await
        .assert_status(201);

    let res = resolve(&root, first, "remove").await;
    res.assert_status(200);
    assert_eq!(res.json()["status"], "removed");
    assert_eq!(res.json()["book"]["title"], "[removed]");
    assert_eq!(
        root.get("/api/admin/reports").await.json()["items"],
        json!([]),
        "bob's report is resolved with it"
    );

    let detail = ada.get(&format!("/api/books/works/{work}")).await.json();
    assert_eq!(detail["title"], "[removed]");
    assert_eq!(detail["subtitle"], "");
    assert_eq!(detail["authors"], "");
    assert_eq!(detail["description"], "");
    assert_eq!(detail["subjects"], json!([]));
    let sources: Value = sqlx::query_scalar("SELECT field_sources FROM works WHERE id = $1")
        .bind(work)
        .fetch_one(&app.db)
        .await
        .unwrap();
    for field in [
        "title",
        "subtitle",
        "author_names",
        "description",
        "subjects",
    ] {
        assert_eq!(sources[field], "admin", "{field}");
    }
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM shelf_entries WHERE work_id = {work}"
        ))
        .await,
        1,
        "the shelf keeps the book"
    );

    let to_ada = mails_to(&app, "ada");
    assert_eq!(to_ada.len(), 1);
    assert_eq!(to_ada[0].subject, "Your report on Bookclub: removed");
    assert!(
        to_ada[0]
            .text
            .contains("reviewed the book \"My Zine\" you reported and removed it")
    );
    assert_eq!(mails_to(&app, "bob").len(), 1);
    let to_carl = mails_to(&app, "carl");
    assert_eq!(to_carl.len(), 1);
    assert_eq!(
        to_carl[0].subject,
        "A book you added on Bookclub was removed"
    );
    assert!(
        to_carl[0]
            .text
            .contains("removed the book \"My Zine\" you added.")
    );
    assert!(to_carl[0].text.contains("because it is offensive"));
    for secret in ["ada", "bob", "Ada here"] {
        assert!(!to_carl[0].text.contains(secret), "{secret}");
    }
    assert!(!to_ada[0].text.contains("carl"));

    // A later report of the removed book is closed on arrival, like a report of a picture taken
    // down already: it never puts the book back in the queue (E2E check of the ADM2 merge), a
    // repeat names it again, and nobody is mailed.
    let dan = app.user("dan").await;
    let later = report_work(&dan, work, json!({"reason": "other"})).await;
    later.assert_status(201);
    assert_eq!(later.json()["status"], "removed");
    let later = later.json()["id"].as_i64().unwrap();
    let again = report_work(&dan, work, json!({"reason": "offensive"})).await;
    assert_eq!(again.json(), json!({"id": later, "status": "removed"}));
    // Ada's own report was decided: a repeat names it, as for a picture taken down.
    let hers = report_work(&ada, work, json!({"reason": "other"})).await;
    assert_eq!(hers.json(), json!({"id": first, "status": "removed"}));
    assert_eq!(
        root.get("/api/admin/reports?status=open").await.json()["items"],
        json!([])
    );
    assert_eq!(
        root.get("/api/admin/summary").await.json()["reports"]["open"],
        0
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM reports WHERE id = {later} AND closed_on_arrival
               AND resolved_by IS NULL"
        ))
        .await,
        1
    );
    assert_eq!(mails_to(&app, "ada").len(), 1);
    assert!(mails_to(&app, "dan").is_empty());

    // A report that reaches it anyway (open since before): removing again changes nothing and
    // sends no second statement.
    let stray = app
        .scalar_i64(&format!(
            "INSERT INTO reports (target_kind, work_id, target_work_id, reason, created_by)
             VALUES ('work', {work}, {work}, 'other', {}) RETURNING id",
            bob.id()
        ))
        .await;
    resolve(&root, stray, "remove").await.assert_status(200);
    assert_eq!(mails_to(&app, "bob").len(), 2);
    assert_eq!(mails_to(&app, "carl").len(), 1);

    // An admin who gives it text again makes it reportable again.
    root.patch(
        &format!("/api/admin/works/{work}"),
        json!({"fields": {"title": "My Zine, cleaned"}}),
    )
    .await
    .assert_status(200);
    let open = report_work(&bob, work, json!({"reason": "other"})).await;
    assert_eq!(open.json()["status"], "open");
}

/// An edition a reader added: "remove" drops its own title, subtitle and publisher (it shows its
/// work's title then); "dismiss" keeps everything and mails only the reporter.
#[sqlx::test]
async fn editions_are_reported_and_decided(db: PgPool) {
    let app = TestApp::new(db).await;
    let carl = app.user("carl").await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    app.exec(&format!(
        "UPDATE users SET locale = 'de' WHERE id = {}",
        carl.id()
    ))
    .await;
    let (work, edition) = zine(&app, &carl).await;
    app.exec(&format!(
        "UPDATE editions SET title = 'Mein Heft', subtitle = 'Ausgabe 1' WHERE id = {edition}"
    ))
    .await;

    let kept = report_edition(&ada, edition, json!({"reason": "wrong_book"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let res = resolve(&root, kept, "dismiss").await;
    res.assert_status(200);
    assert_eq!(res.json()["status"], "dismissed");
    assert_eq!(res.json()["target_kind"], "edition");
    assert_eq!(res.json()["book"]["id"], work);
    assert_eq!(res.json()["book"]["title"], "Mein Heft");
    let to_ada = mails_to(&app, "ada");
    assert_eq!(to_ada.len(), 1);
    assert_eq!(to_ada[0].subject, "Your report on Bookclub: not removed");
    assert!(
        to_ada[0]
            .text
            .contains("the edition of \"Mein Heft\" you reported")
    );
    assert!(
        mails_to(&app, "carl").is_empty(),
        "no statement for a kept edition"
    );
    let title: Option<String> = sqlx::query_scalar("SELECT title FROM editions WHERE id = $1")
        .bind(edition)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(
        title.as_deref(),
        Some("Mein Heft"),
        "dismiss changes nothing"
    );

    let id = report_edition(&ada, edition, json!({"reason": "copyright"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    assert_ne!(id, kept, "a dismissed report is no open report");
    resolve(&root, id, "remove").await.assert_status(200);
    let row: (Option<String>, Option<String>, String) =
        sqlx::query_as("SELECT title, subtitle, publisher FROM editions WHERE id = $1")
            .bind(edition)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(row, (None, None, String::new()));
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE id = {edition} AND pages = 32"
        ))
        .await,
        1,
        "the facts stay"
    );
    let to_carl = mails_to(&app, "carl");
    assert_eq!(to_carl.len(), 1);
    assert_eq!(
        to_carl[0].subject,
        "Eine Ausgabe, die du bei Bookclub eingetragen hast, wurde entfernt"
    );
    assert!(
        to_carl[0]
            .text
            .contains("die Ausgabe von „Mein Heft“, die du eingetragen hast, entfernt.")
    );
    assert!(
        to_carl[0]
            .text
            .contains("weil er die Urheberrechte anderer verletzt")
    );

    // Its text withdrawn, a new report of the edition is closed on arrival.
    let later = report_edition(&app.user("bob").await, edition, json!({"reason": "other"})).await;
    later.assert_status(201);
    assert_eq!(later.json()["status"], "removed");
    assert_eq!(
        root.get("/api/admin/reports").await.json()["items"],
        json!([])
    );
}

/// Only records readers added can be reported (400); unknown ids are 404, a bad reason 400,
/// anonymous callers 401. A work id follows merges, and the reports share the cover reports'
/// daily limit.
#[sqlx::test]
async fn only_books_readers_added_can_be_reported(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let carl = app.user("carl").await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let edition = app.edition(&circe, "9780316556347").await;
    app.exec(&format!(
        "UPDATE editions SET source = 'dnb' WHERE id = {}",
        edition.id
    ))
    .await;
    report_work(&ada, circe.id, json!({"reason": "other"}))
        .await
        .assert_detail(400, "Only books readers added can be reported");
    report_work(&ada, "OL1W", json!({"reason": "other"}))
        .await
        .assert_detail(400, "Only books readers added can be reported");
    report_edition(&ada, edition.id, json!({"reason": "other"}))
        .await
        .assert_detail(400, "Only books readers added can be reported");
    report_work(&ada, 999_999, json!({"reason": "other"}))
        .await
        .assert_detail(404, "No such book.");
    report_edition(&ada, 999_999, json!({"reason": "other"}))
        .await
        .assert_detail(404, "No such book.");
    let (work, edition) = zine(&app, &carl).await;
    report_work(&ada, work, json!({"reason": "ugly"}))
        .await
        .assert_detail(
            400,
            "Input should be 'copyright', 'wrong_book', 'offensive' or 'other'",
        );
    report_edition(&app.anon(), edition, json!({"reason": "other"}))
        .await
        .assert_detail(401, "Not signed in");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 0);

    // A user-added work merged into another user-added work: the report names the survivor.
    let survivor = app
        .scalar_i64(&format!(
            "INSERT INTO works (title, source, created_by) VALUES ('Zine, merged', 'user', {})
             RETURNING id",
            carl.id()
        ))
        .await;
    app.exec(&format!(
        "UPDATE works SET merged_into_id = {survivor} WHERE id = {work}"
    ))
    .await;
    let id = report_work(&ada, work, json!({"reason": "other"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        app.scalar_i64(&format!("SELECT work_id FROM reports WHERE id = {id}"))
            .await,
        survivor
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT target_work_id FROM reports WHERE id = {id}"
        ))
        .await,
        survivor,
        "the reported record is the survivor"
    );

    // 20 new reports a day, shared with cover and avatar reports: 19 more works, then the
    // 21st report of any kind is refused.
    app.exec(&format!(
        "INSERT INTO works (title, source, created_by)
         SELECT 'Zine ' || n, 'user', {} FROM generate_series(1, 19) AS n",
        carl.id()
    ))
    .await;
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM works WHERE title LIKE 'Zine %' ORDER BY id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    for id in ids.iter().filter(|id| **id != survivor) {
        report_work(&ada, id, json!({"reason": "other"}))
            .await
            .assert_status(201);
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 20);
    report_edition(&ada, edition, json!({"reason": "other"}))
        .await
        .assert_detail(429, "Slow down a little. Try again later.");
}

/// The text of a work, as a decision could change it: title, subtitle, authors (and the author
/// links), description, subjects and their provenance.
async fn work_text(app: &TestApp, work: i64) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
                    'title', title, 'subtitle', subtitle, 'authors', author_names,
                    'description', description, 'subjects', subjects,
                    'sources', field_sources,
                    'author_links', (SELECT count(*) FROM work_authors WHERE work_id = w.id))
         FROM works w WHERE id = $1",
    )
    .bind(work)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// A merge after the report re-points it to the surviving work (`catalog::merge`), here an Open
/// Library work on every reader's shelf. The queue shows the survivor and flags the report
/// `gone`; "remove" is refused and changes nothing on the survivor, mails nobody; "dismiss"
/// closes both reports of the merged book, and its reporters learn that it was merged. The reader
/// who added it gets no statement.
#[sqlx::test]
async fn a_merge_after_the_report_never_hands_the_survivor_to_remove(db: PgPool) {
    let app = TestApp::new(db).await;
    let carl = app.user("carl").await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let root = app.admin("root").await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    app.exec(&format!(
        "UPDATE works SET subtitle = 'A Novel', description = 'A witch on an island.',
                          subjects = ARRAY['Myth']
         WHERE id = {}",
        circe.id
    ))
    .await;
    let (zine, _) = zine(&app, &carl).await;
    let first = report_work(&ada, zine, json!({"reason": "offensive"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    report_work(&bob, zine, json!({"reason": "other"}))
        .await
        .assert_status(201);

    root.post(
        &format!("/api/admin/works/{zine}/merge"),
        json!({"into_work_id": circe.id}),
    )
    .await
    .assert_status(200);
    let before = work_text(&app, circe.id).await;
    let items = root.get("/api/admin/reports").await.json()["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 2);
    for item in items.as_array().unwrap() {
        assert_eq!(item["target_kind"], "work");
        assert_eq!(item["book"]["id"], circe.id, "the book readers see now");
        assert_eq!(item["gone"], true);
    }

    resolve(&root, first, "remove")
        .await
        .assert_detail(409, "The reported book was merged into another one");
    assert_eq!(
        work_text(&app, circe.id).await,
        before,
        "Circe is untouched"
    );
    assert_eq!(before["title"], "Circe");
    assert_eq!(before["authors"], "Madeline Miller");
    assert_eq!(before["author_links"], 1);
    for (field, source) in before["sources"].as_object().unwrap() {
        assert_ne!(source, "admin", "{field}");
    }
    assert!(app.sent_emails().is_empty(), "{:#?}", app.sent_emails());
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM reports WHERE status = 'open'")
            .await,
        2
    );

    let res = resolve(&root, first, "dismiss").await;
    res.assert_status(200);
    assert_eq!(res.json()["status"], "dismissed");
    assert_eq!(res.json()["gone"], true);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM reports WHERE status = 'open'")
            .await,
        0,
        "bob's report closed with ada's"
    );
    assert_eq!(work_text(&app, circe.id).await, before);
    for reporter in ["ada", "bob"] {
        let mails = mails_to(&app, reporter);
        assert_eq!(mails.len(), 1, "{reporter}");
        assert_eq!(mails[0].subject, "Your report on Bookclub: merged");
        assert!(
            mails[0].text.contains(
                "reviewed the book \"My Zine\" you reported: it no longer shows on its own"
            ),
            "{}",
            mails[0].text
        );
    }
    assert!(mails_to(&app, "carl").is_empty(), "no statement of reasons");
}

/// A reader's book merged into another reader's: each report stays with the record it named.
/// A report of the survivor removes only the survivor's text (its reader gets the statement),
/// and does not close the report of the merged book, which "remove" refuses; a repeat names the
/// reported record, not the book shown.
#[sqlx::test]
async fn a_merge_into_another_readers_book_keeps_the_reports_apart(db: PgPool) {
    let app = TestApp::new(db).await;
    let carl = app.user("carl").await;
    let dora = app.user("dora").await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let root = app.admin("root").await;
    let (zine, _) = zine(&app, &carl).await;
    let res = dora
        .post(
            "/api/books/custom",
            json!({"title": "Dora's Zine", "authors": "Dora D", "description": "Her blurb."}),
        )
        .await;
    res.assert_status(201);
    let doras = res.json()["id"].as_i64().unwrap();
    let of_carls = report_work(&ada, zine, json!({"reason": "offensive"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    root.post(
        &format!("/api/admin/works/{zine}/merge"),
        json!({"into_work_id": doras}),
    )
    .await
    .assert_status(200);
    let carls_text = work_text(&app, zine).await;

    let of_doras = report_work(&bob, doras, json!({"reason": "copyright"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let adas_second = report_work(&ada, doras, json!({"reason": "other"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    assert_ne!(
        adas_second, of_carls,
        "a report of dora's book is a new one"
    );

    resolve(&root, of_doras, "remove").await.assert_status(200);
    assert_eq!(work_text(&app, doras).await["title"], "[removed]");
    let open: Vec<i64> = sqlx::query_scalar("SELECT id FROM reports WHERE status = 'open'")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(open, [of_carls], "ada's report of dora's book closed too");
    let to_dora = mails_to(&app, "dora");
    assert_eq!(to_dora.len(), 1);
    assert_eq!(
        to_dora[0].subject,
        "A book you added on Bookclub was removed"
    );
    assert!(to_dora[0].text.contains("\"Dora's Zine\""));
    assert!(mails_to(&app, "carl").is_empty());

    resolve(&root, of_carls, "remove")
        .await
        .assert_detail(409, "The reported book was merged into another one");
    resolve(&root, of_carls, "dismiss").await.assert_status(200);
    assert_eq!(
        work_text(&app, zine).await,
        carls_text,
        "carl's record as it was"
    );
    let to_ada = mails_to(&app, "ada");
    assert_eq!(to_ada.len(), 2);
    assert_eq!(to_ada[0].subject, "Your report on Bookclub: removed");
    assert_eq!(to_ada[1].subject, "Your report on Bookclub: merged");
    assert!(mails_to(&app, "carl").is_empty(), "no statement to carl");
}

/// Waits until `sessions` connections wait for a lock (false after ten seconds).
async fn lock_waits(db: &PgPool, sessions: i64) -> bool {
    for _ in 0..500 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        if waiting >= sessions {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// Two identical reports of a book (or an edition) by one reader at once are one report: the
/// advisory lock on (reader, record) makes the second wait until the first is committed, and
/// then it finds that one. Without the lock both would find no open report and both would
/// insert, and both would count against the daily limit.
#[sqlx::test]
async fn two_reports_of_a_book_at_once_by_one_reader_are_one_report(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let carl = app.user("carl").await;
    let (work, edition) = zine(&app, &carl).await;
    let a = ada.id();
    for (path, key) in [
        (
            format!("/api/books/works/{work}/reports"),
            format!("{a}:work:{work}"),
        ),
        (
            format!("/api/books/editions/{edition}/reports"),
            format!("{a}:edition:{edition}"),
        ),
    ] {
        // The server's lock: space 0x6263_0200, key hashtext('<user id>:<kind>:<id>').
        let mut holder = app.db.acquire().await.unwrap();
        sqlx::query("SELECT pg_advisory_lock($1, hashtext($2))")
            .bind(0x6263_0200_i32)
            .bind(&key)
            .execute(&mut *holder)
            .await
            .unwrap();
        let release = async {
            assert!(
                lock_waits(&app.db, 2).await,
                "both wait for the reader's turn"
            );
            sqlx::query("SELECT pg_advisory_unlock($1, hashtext($2))")
                .bind(0x6263_0200_i32)
                .bind(&key)
                .execute(&mut *holder)
                .await
                .unwrap();
        };
        let body = json!({"reason": "offensive"});
        let (first, second, ()) = tokio::join!(
            ada.post(&path, body.clone()),
            ada.post(&path, body.clone()),
            release
        );
        first.assert_status(201);
        second.assert_status(201);
        assert_eq!(first.json()["status"], "open");
        assert_eq!(first.json(), second.json(), "{path}: the same report");
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 2);
}

/// Every report request counts against the per-account limit of 30 a minute, shared with the
/// picture reports: repeats, which the daily limit never counts, and refusals too. Another
/// reader is not affected.
#[sqlx::test]
async fn every_book_report_request_counts_against_a_minute_limit(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let carl = app.user("carl").await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let (work, edition) = zine(&app, &carl).await;
    let id = report_work(&ada, work, json!({"reason": "offensive"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    report_work(&ada, circe.id, json!({"reason": "other"}))
        .await
        .assert_detail(400, "Only books readers added can be reported");
    for _ in 0..28 {
        let again = report_work(&ada, work, json!({"reason": "other"})).await;
        assert_eq!(again.json(), json!({"id": id, "status": "open"}));
    }
    let limited = report_edition(&ada, edition, json!({"reason": "other"})).await;
    limited.assert_detail(429, "Slow down a little. Try again later.");
    let wait: u64 = limited.header("retry-after").unwrap().parse().unwrap();
    assert!((1..=60).contains(&wait), "{wait}");
    report_work(&ada, work, json!({"reason": "other"}))
        .await
        .assert_detail(429, "Slow down a little. Try again later.");
    report_edition(&grace, edition, json!({"reason": "other"}))
        .await
        .assert_status(201);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 2);
}

/// The book page shows a withdrawn book's stored placeholder title translated, and finds it by
/// the SPA's copy of the server's constant (`frontend/src/components/ReportSheet.vue`).
#[test]
fn the_spa_knows_the_removed_title() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../frontend/src/components/ReportSheet.vue");
    let sheet = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let line = format!(
        "export const REMOVED_TITLE = {:?};",
        bookclub_server::catalog::reports::REMOVED_TITLE
    );
    assert!(sheet.contains(&line), "{line} is missing from {path:?}");
}
