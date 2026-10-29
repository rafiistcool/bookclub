//! Reports on works and editions readers added (#54, Q11 option B):
//! `POST /api/books/works/{work_id}/reports`, `POST /api/books/editions/{edition_id}/reports`,
//! and their decisions in the one admin queue, with the decision notices (Q12 option A).

mod common;

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

    // A later report of the removed book is a new open report; removing again changes nothing
    // and sends no second statement.
    let later = report_work(&ada, work, json!({"reason": "other"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    assert_ne!(later, first);
    resolve(&root, later, "remove").await.assert_status(200);
    assert_eq!(mails_to(&app, "ada").len(), 2);
    assert_eq!(mails_to(&app, "carl").len(), 1);
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
