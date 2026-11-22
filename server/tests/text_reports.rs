//! Reports on texts (#55, Q11 option C): posts, quotes and takes, filed through
//! `POST /api/posts/{post_id}/reports`, `POST /api/quotes/{quote_id}/reports` and
//! `POST /api/books/works/{work_id}/takes/{username}/reports`, and decided in the one admin queue
//! with the decision notices (docs/phase3/notes/texts.md).

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// `ada` writes; `bob` and `dan` share the club `Readers` with her (Circe is its pick); `carl`
/// is a stranger; `root` is an instance admin.
struct World {
    app: TestApp,
    ada: TestClient,
    bob: TestClient,
    carl: TestClient,
    dan: TestClient,
    root: TestClient,
    club: TestClub,
    circe: TestWork,
}

async fn world(db: PgPool) -> World {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let carl = app.user("carl").await;
    let dan = app.user("dan").await;
    let root = app.admin("root").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &bob, "member").await;
    app.join(&club, &dan, "member").await;
    let circe = app.work("Circe").await;
    sqlx::query("INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2)")
        .bind(club.id)
        .bind(circe.id)
        .execute(&app.db)
        .await
        .unwrap();
    World {
        app,
        ada,
        bob,
        carl,
        dan,
        root,
        club,
        circe,
    }
}

impl World {
    /// A diary entry (or reply) on Circe; returns its id.
    async fn diary(&self, who: &TestClient, body: Value) -> i64 {
        let res = who
            .post(&format!("/api/books/works/{}/posts", self.circe.id), body)
            .await;
        res.assert_status(201);
        res.json()["id"].as_i64().unwrap()
    }

    /// A club post (or reply) on the current pick; returns its id.
    async fn club_post(&self, who: &TestClient, body: Value) -> i64 {
        let res = who
            .post(&format!("/api/clubs/{}/pick/posts", self.club.slug), body)
            .await;
        res.assert_status(201);
        res.json()["id"].as_i64().unwrap()
    }

    async fn quote(&self, who: &TestClient, body: &str, visibility: &str) -> i64 {
        let res = who
            .post(
                "/api/quotes",
                json!({"work_id": self.circe.id, "body": body, "visibility": visibility}),
            )
            .await;
        res.assert_status(201);
        res.json()["id"].as_i64().unwrap()
    }

    /// Circe finished on `who`'s shelf, rated 4, with `take`.
    async fn take(&self, who: &TestClient, take: &str, visibility: &str) {
        who.post(
            "/api/shelf",
            json!({"work_id": self.circe.id, "status": "finished", "rating": 4, "take": take,
                   "rating_visibility": visibility}),
        )
        .await
        .assert_status(201);
    }

    fn take_path(&self, username: &str) -> String {
        format!(
            "/api/books/works/{}/takes/{username}/reports",
            self.circe.id
        )
    }

    async fn open_reports(&self) -> Vec<Value> {
        self.root.get("/api/admin/reports").await.json()["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn mails_to(&self, username: &str) -> Vec<bookclub_server::email::Email> {
        self.app.email.sent_to(&format!("{username}@example.test"))
    }
}

async fn report(who: &TestClient, path: &str, body: Value) -> TestResponse {
    who.post(path, body).await
}

fn post_path(id: i64) -> String {
    format!("/api/posts/{id}/reports")
}

fn quote_path(id: i64) -> String {
    format!("/api/quotes/{id}/reports")
}

async fn resolve(admin: &TestClient, report: i64, action: &str) -> TestResponse {
    admin
        .post(
            &format!("/api/admin/reports/{report}/resolve"),
            json!({ "action": action }),
        )
        .await
}

async fn export(who: &TestClient) -> TestResponse {
    let res = who.get("/api/auth/me/export").await;
    res.assert_status(200);
    res
}

/// `data.json` of an export.
fn data_json(res: &TestResponse) -> Value {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(res.body.to_vec())).expect("a ZIP");
    let file = archive.by_name("data.json").expect("data.json");
    serde_json::from_reader(file).unwrap()
}

fn id_of(res: &TestResponse) -> i64 {
    res.assert_status(201);
    res.json()["id"].as_i64().unwrap()
}

/// Acceptance: a club mate reports a diary entry; the queue shows the text as reported, the text
/// as it is now, the author and the book; an edit since shows as the current text; a repeat
/// names the same report. A club post names its club.
#[sqlx::test]
async fn a_post_report_shows_the_snapshot_and_the_current_text(db: PgPool) {
    let w = world(db).await;
    let entry = w.diary(&w.ada, json!({"body": "Circe is a bore"})).await;
    let res = report(
        &w.bob,
        &post_path(entry),
        json!({"reason": "offensive", "details": "Rude"}),
    )
    .await;
    let id = id_of(&res);
    assert_eq!(res.json()["status"], "open");
    let again = report(&w.bob, &post_path(entry), json!({"reason": "other"})).await;
    assert_eq!(id_of(&again), id);

    w.ada
        .patch(
            &format!("/api/posts/{entry}"),
            json!({"body": "Circe is a delight"}),
        )
        .await
        .assert_status(200);
    let items = w.open_reports().await;
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item["id"], id);
    assert_eq!(item["target_kind"], "post");
    assert_eq!(item["reason"], "offensive");
    assert_eq!(item["details"], "Rude");
    assert_eq!(item["text_snapshot"], "Circe is a bore");
    assert_eq!(item["current_text"], "Circe is a delight");
    assert_eq!(item["target_user"]["username"], "ada");
    assert_eq!(item["created_by"]["username"], "bob");
    assert_eq!(item["book"]["id"], w.circe.id);
    assert_eq!(item["club"], Value::Null);
    assert_eq!(item["gone"], false);
    assert_eq!(item["media_public_id"], Value::Null);

    let club_post = w.club_post(&w.ada, json!({"body": "Meet at eight"})).await;
    id_of(&report(&w.bob, &post_path(club_post), json!({"reason": "other"})).await);
    let items = w.open_reports().await;
    assert_eq!(items[1]["club"]["name"], "Readers");
    assert_eq!(items[1]["club"]["slug"], w.club.slug.as_str());
    assert_eq!(
        w.root.get("/api/admin/summary").await.json()["reports"]["open"],
        2
    );
}

/// The reporter must see the text through the visibility policy, else 404 like an unknown id;
/// a tombstone and an empty take are not reportable; one's own text is 400, and so is
/// `wrong_book`.
#[sqlx::test]
async fn text_reports_follow_the_visibility_policy(db: PgPool) {
    let w = world(db).await;
    let open = w.diary(&w.ada, json!({"body": "Open words"})).await;
    let secret = w
        .diary(
            &w.ada,
            json!({"body": "Secret words", "visibility": "only_me"}),
        )
        .await;
    let club_post = w.club_post(&w.ada, json!({"body": "Club words"})).await;
    let open_quote = w.quote(&w.ada, "A quoted line", "audience").await;
    let secret_quote = w.quote(&w.ada, "A private line", "only_me").await;
    w.take(&w.ada, "Loved it", "audience").await;
    let body = json!({"reason": "offensive"});

    // A stranger sees none of it.
    for path in [post_path(open), post_path(club_post)] {
        report(&w.carl, &path, body.clone())
            .await
            .assert_detail(404, "No entry with that id");
    }
    report(&w.carl, &quote_path(open_quote), body.clone())
        .await
        .assert_detail(404, "No quote with that id");
    report(&w.carl, &w.take_path("ada"), body.clone())
        .await
        .assert_detail(404, "Not found");
    // A club mate does not see what ada keeps to herself.
    report(&w.bob, &post_path(secret), body.clone())
        .await
        .assert_detail(404, "No entry with that id");
    report(&w.bob, &quote_path(secret_quote), body.clone())
        .await
        .assert_detail(404, "No quote with that id");
    // Unknown ids and names answer the same.
    report(&w.bob, &post_path(999_999), body.clone())
        .await
        .assert_detail(404, "No entry with that id");
    report(&w.bob, &quote_path(999_999), body.clone())
        .await
        .assert_detail(404, "No quote with that id");
    report(&w.bob, &w.take_path("nobody"), body.clone())
        .await
        .assert_detail(404, "Not found");
    report(&w.bob, &w.take_path("dan"), body.clone())
        .await
        .assert_detail(404, "Not found");
    report(
        &w.bob,
        "/api/books/works/999999/takes/ada/reports",
        body.clone(),
    )
    .await
    .assert_detail(404, "No such book.");

    // A non-member does not see a club post, even as ada's friend-of-a-club elsewhere.
    let other = w.app.club(&w.carl, "Elsewhere").await;
    w.app.join(&other, &w.ada, "member").await;
    report(&w.carl, &post_path(club_post), body.clone())
        .await
        .assert_detail(404, "No entry with that id");
    // ...while ada's diary entry now shows to carl, and can be reported.
    id_of(&report(&w.carl, &post_path(open), body.clone()).await);

    // An only-me take: the rating and take hide from bob, so does the take's report.
    w.ada
        .patch(
            &format!(
                "/api/shelf/{}",
                w.app
                    .scalar_i64(&format!(
                        "SELECT id FROM shelf_entries WHERE user_id = {}",
                        w.ada.id()
                    ))
                    .await
            ),
            json!({"rating_visibility": "only_me"}),
        )
        .await
        .assert_status(200);
    report(&w.bob, &w.take_path("ada"), body.clone())
        .await
        .assert_detail(404, "Not found");
    // An empty take: nothing to report.
    w.take(&w.dan, "", "audience").await;
    report(&w.bob, &w.take_path("dan"), body.clone())
        .await
        .assert_detail(404, "Not found");

    // A tombstone (deleted, with a reply) is as missing.
    let root_post = w.diary(&w.ada, json!({"body": "Root"})).await;
    w.diary(&w.bob, json!({"body": "Reply", "parent_id": root_post}))
        .await;
    w.ada
        .delete(&format!("/api/posts/{root_post}"))
        .await
        .assert_status(204);
    report(&w.dan, &post_path(root_post), body.clone())
        .await
        .assert_detail(404, "No entry with that id");

    // One's own text.
    report(&w.ada, &post_path(open), body.clone())
        .await
        .assert_detail(400, "You cannot report your own text");
    report(&w.ada, &quote_path(open_quote), body.clone())
        .await
        .assert_detail(400, "You cannot report your own text");
    w.take(&w.bob, "Mine", "audience").await;
    report(&w.bob, &w.take_path("BOB"), body.clone())
        .await
        .assert_detail(400, "You cannot report your own text");
    // The wrong book is a reason for covers and books, and the reason list is checked first.
    report(&w.bob, &post_path(open), json!({"reason": "wrong_book"}))
        .await
        .assert_detail(400, "A text cannot be reported as the wrong book");
    report(
        &w.bob,
        &w.take_path("nobody"),
        json!({"reason": "wrong_book"}),
    )
    .await
    .assert_detail(400, "A text cannot be reported as the wrong book");
    report(&w.bob, &post_path(open), json!({"reason": "spam"}))
        .await
        .assert_detail(
            400,
            "Input should be 'copyright', 'wrong_book', 'offensive' or 'other'",
        );
    assert_eq!(
        w.app
            .scalar_i64("SELECT count(*) FROM reports WHERE created_by <> (SELECT id FROM users WHERE username = 'carl')")
            .await,
        0
    );
}

/// "Remove" per kind: a post with replies becomes a tombstone (its replies stay), a quote is
/// deleted, a take is cleared with its rating kept; each is gone from every read, the feeds'
/// stored copies too. Every open report of the text is resolved together; the reporters learn
/// the decision and the author gets a statement of reasons that quotes their text, neither naming
/// the other.
#[sqlx::test]
async fn remove_withdraws_each_kind_of_text(db: PgPool) {
    let w = world(db).await;
    let post = w.club_post(&w.ada, json!({"body": "Nasty words"})).await;
    let reply = w
        .club_post(&w.dan, json!({"body": "Calm down", "parent_id": post}))
        .await;
    let quote = w.quote(&w.ada, "A copied page", "audience").await;
    w.take(&w.ada, "Trash take", "audience").await;

    let first = id_of(
        &report(
            &w.bob,
            &post_path(post),
            json!({"reason": "offensive", "details": "Bob here"}),
        )
        .await,
    );
    id_of(&report(&w.dan, &post_path(post), json!({"reason": "other"})).await);
    let res = resolve(&w.root, first, "remove").await;
    res.assert_status(200);
    assert_eq!(res.json()["status"], "removed");
    assert_eq!(res.json()["current_text"], Value::Null);
    assert_eq!(res.json()["text_snapshot"], "Nasty words");
    assert_eq!(res.json()["gone"], false);
    assert!(
        w.open_reports().await.is_empty(),
        "dan's report goes with it"
    );
    let thread = w
        .bob
        .get(&format!("/api/clubs/{}/pick/posts", w.club.slug))
        .await
        .json();
    let items = thread["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["deleted"], true);
    assert_eq!(items[0]["body"], "");
    assert_eq!(items[0]["replies"][0]["id"], reply);
    assert_eq!(items[0]["replies"][0]["body"], "Calm down");
    for name in ["bob", "dan"] {
        let mails = w.mails_to(name);
        assert_eq!(mails.len(), 1, "{name}");
        assert_eq!(mails[0].subject, "Your report on Bookclub: removed");
        assert!(
            mails[0]
                .text
                .contains("reviewed the post on \"Circe\" you reported and removed it."),
            "{}",
            mails[0].text
        );
        assert!(!mails[0].text.contains("ada"));
        assert!(!mails[0].text.contains("Nasty words"));
    }
    let to_ada = w.mails_to("ada");
    assert_eq!(to_ada.len(), 1);
    assert_eq!(to_ada[0].subject, "Your post on Bookclub was removed");
    assert!(
        to_ada[0]
            .text
            .contains("removed your post on \"Circe\".\n\nThe removed text read: \"Nasty words\"")
    );
    assert!(to_ada[0].text.contains("because it is offensive"));
    for secret in ["bob", "dan", "Bob here"] {
        assert!(!to_ada[0].text.contains(secret), "{secret}");
    }

    // A reply without replies of its own is deleted, as its author's delete would.
    let lone = w.diary(&w.ada, json!({"body": "Lone words"})).await;
    let lone_report = id_of(&report(&w.bob, &post_path(lone), json!({"reason": "other"})).await);
    resolve(&w.root, lone_report, "remove")
        .await
        .assert_status(200);
    assert_eq!(
        w.app
            .scalar_i64(&format!("SELECT count(*) FROM posts WHERE id = {lone}"))
            .await,
        0
    );

    let quote_report =
        id_of(&report(&w.bob, &quote_path(quote), json!({"reason": "copyright"})).await);
    resolve(&w.root, quote_report, "remove")
        .await
        .assert_status(200);
    let quotes = w
        .bob
        .get(&format!("/api/quotes?work={}", w.circe.id))
        .await
        .json();
    assert_eq!(quotes["items"], json!([]));
    let ada_quotes = w
        .ada
        .get(&format!("/api/quotes?work={}", w.circe.id))
        .await
        .json();
    assert_eq!(ada_quotes["items"], json!([]));
    assert!(
        w.mails_to("ada")
            .last()
            .unwrap()
            .text
            .contains("The removed text read: \"A copied page\"")
    );

    let take_report =
        id_of(&report(&w.dan, &w.take_path("ada"), json!({"reason": "offensive"})).await);
    resolve(&w.root, take_report, "remove")
        .await
        .assert_status(200);
    let shelf = w.bob.get("/api/shelf?username=ada").await.json();
    let entry = &shelf["items"][0];
    assert_eq!(entry["take"], "");
    assert_eq!(entry["rating"], 4);
    let pick = w
        .bob
        .get(&format!("/api/clubs/{}/pick", w.club.slug))
        .await
        .json();
    for reader in pick["pick"]["readers"].as_array().unwrap() {
        assert_eq!(reader["take"], "", "{reader}");
    }
    // Every copy in the feeds: ada's own and her club mates'.
    for who in [&w.ada, &w.bob] {
        let feed = who.get("/api/activity?limit=50").await.json();
        let text = feed.to_string();
        for gone in ["Trash take", "A copied page", "Nasty words"] {
            assert!(!text.contains(gone), "{gone} in {text}");
        }
    }
    let to_ada = w.mails_to("ada");
    assert_eq!(to_ada.len(), 4);
    assert_eq!(
        to_ada.last().unwrap().subject,
        "Your take on a book on Bookclub was removed"
    );
    assert!(to_ada.last().unwrap().text.contains("your rating stays"));

    // History: every removal, decided by root.
    let history = w
        .root
        .get("/api/admin/reports?status=resolved")
        .await
        .json();
    let kinds: Vec<&str> = history["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["target_kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["take", "quote", "post", "post", "post"]);
    for item in history["items"].as_array().unwrap() {
        assert_eq!(item["status"], "removed");
        assert_eq!(item["resolved_by"]["username"], "root");
        assert_eq!(item["gone"], false);
    }
}

/// "Dismiss" keeps the text and mails only the reporter.
#[sqlx::test]
async fn dismiss_keeps_the_text(db: PgPool) {
    let w = world(db).await;
    let quote = w.quote(&w.ada, "A fine line", "audience").await;
    let id = id_of(&report(&w.bob, &quote_path(quote), json!({"reason": "other"})).await);
    let res = resolve(&w.root, id, "dismiss").await;
    res.assert_status(200);
    assert_eq!(res.json()["status"], "dismissed");
    assert_eq!(res.json()["current_text"], "A fine line");
    assert_eq!(
        w.bob
            .get(&format!("/api/quotes?work={}", w.circe.id))
            .await
            .json()["items"][0]["body"],
        "A fine line"
    );
    let to_bob = w.mails_to("bob");
    assert_eq!(to_bob.len(), 1);
    assert_eq!(to_bob[0].subject, "Your report on Bookclub: not removed");
    assert!(w.mails_to("ada").is_empty());
    resolve(&w.root, id, "remove")
        .await
        .assert_detail(409, "This report was already resolved");
    // A report after a dismissal is a new one.
    let again = id_of(&report(&w.bob, &quote_path(quote), json!({"reason": "other"})).await);
    assert_ne!(again, id);
}

/// A text its author deleted since is gone: "remove" is 409, "dismiss" closes the report and
/// tells the reporter it is no longer online; the author gets nothing.
#[sqlx::test]
async fn a_text_its_author_deleted_is_gone(db: PgPool) {
    let w = world(db).await;
    let post = w.diary(&w.ada, json!({"body": "Soon gone"})).await;
    w.take(&w.ada, "Soon cleared", "audience").await;
    let post_report = id_of(&report(&w.bob, &post_path(post), json!({"reason": "other"})).await);
    let take_report = id_of(&report(&w.bob, &w.take_path("ada"), json!({"reason": "other"})).await);
    w.ada
        .delete(&format!("/api/posts/{post}"))
        .await
        .assert_status(204);
    let entry = w
        .app
        .scalar_i64(&format!(
            "SELECT id FROM shelf_entries WHERE user_id = {}",
            w.ada.id()
        ))
        .await;
    w.ada
        .patch(&format!("/api/shelf/{entry}"), json!({"take": ""}))
        .await
        .assert_status(200);

    let items = w.open_reports().await;
    assert_eq!(items.len(), 2);
    for item in &items {
        assert_eq!(item["gone"], true, "{item}");
        assert_eq!(item["current_text"], Value::Null);
    }
    assert_eq!(items[0]["text_snapshot"], "Soon gone");
    for id in [post_report, take_report] {
        resolve(&w.root, id, "remove")
            .await
            .assert_detail(409, "The reported text is no longer online");
    }
    assert!(w.app.email.sent().is_empty());
    for id in [post_report, take_report] {
        resolve(&w.root, id, "dismiss").await.assert_status(200);
    }
    let to_bob = w.mails_to("bob");
    assert_eq!(to_bob.len(), 2);
    assert!(to_bob[0].subject.ends_with("no longer online"));
    assert!(
        to_bob[0]
            .text
            .contains("it is no longer online, because it was deleted.")
    );
    assert!(w.mails_to("ada").is_empty());
}

/// A text an admin removed is answered with a report closed on arrival (the reader's view was
/// older than the decision); it never enters the queue, and nobody is mailed. A take written
/// again is reportable again.
#[sqlx::test]
async fn a_removed_text_closes_later_reports_on_arrival(db: PgPool) {
    let w = world(db).await;
    let post = w.diary(&w.ada, json!({"body": "Bad root"})).await;
    w.diary(&w.dan, json!({"body": "A reply", "parent_id": post}))
        .await;
    w.take(&w.ada, "Bad take", "audience").await;
    let first = id_of(&report(&w.bob, &post_path(post), json!({"reason": "other"})).await);
    let take = id_of(&report(&w.bob, &w.take_path("ada"), json!({"reason": "other"})).await);
    resolve(&w.root, first, "remove").await.assert_status(200);
    resolve(&w.root, take, "remove").await.assert_status(200);
    let sent = w.app.email.sent().len();

    for path in [post_path(post), w.take_path("ada")] {
        let late = report(&w.dan, &path, json!({"reason": "offensive"})).await;
        late.assert_status(201);
        assert_eq!(late.json()["status"], "removed");
        let again = report(&w.dan, &path, json!({"reason": "offensive"})).await;
        assert_eq!(again.json()["id"], late.json()["id"]);
        assert_eq!(again.json()["status"], "removed");
        // The reporter's own repeat names their removed report.
        let repeat = report(&w.bob, &path, json!({"reason": "other"})).await;
        assert_eq!(repeat.json()["status"], "removed");
    }
    assert!(w.open_reports().await.is_empty());
    assert_eq!(w.app.email.sent().len(), sent);
    assert_eq!(
        w.app
            .scalar_i64(
                "SELECT count(*) FROM reports
                 WHERE closed_on_arrival AND text_snapshot IS NULL AND resolved_by IS NULL"
            )
            .await,
        2
    );

    let entry = w
        .app
        .scalar_i64(&format!(
            "SELECT id FROM shelf_entries WHERE user_id = {}",
            w.ada.id()
        ))
        .await;
    w.ada
        .patch(
            &format!("/api/shelf/{entry}"),
            json!({"take": "Better take"}),
        )
        .await
        .assert_status(200);
    let fresh = report(&w.dan, &w.take_path("ada"), json!({"reason": "other"})).await;
    fresh.assert_status(201);
    assert_eq!(fresh.json()["status"], "open");
}

/// The author's export lists the removal of their text, with their own text as reported; the
/// reporter's lists the report without the other reader's text.
#[sqlx::test]
async fn the_export_shows_the_author_the_removal_and_the_reporter_no_text(db: PgPool) {
    let w = world(db).await;
    let quote = w.quote(&w.ada, "Exported words", "audience").await;
    let id = id_of(&report(&w.bob, &quote_path(quote), json!({"reason": "copyright"})).await);
    resolve(&w.root, id, "remove").await.assert_status(200);

    let ada = data_json(&export(&w.ada).await);
    let about = ada["reports"]["about_me"].as_array().unwrap();
    assert_eq!(about.len(), 1);
    assert_eq!(about[0]["target_kind"], "quote");
    assert_eq!(about[0]["work_id"], w.circe.id);
    assert_eq!(about[0]["text_snapshot"], "Exported words");
    assert_eq!(about[0]["reason"], "copyright");
    for hidden in ["created_by", "details", "created_at", "id", "via"] {
        assert!(about[0].get(hidden).is_none(), "{hidden}");
    }

    let bob = data_json(&export(&w.bob).await);
    let filed = bob["reports"]["filed"].as_array().unwrap();
    assert_eq!(filed.len(), 1);
    assert_eq!(filed[0]["target_kind"], "quote");
    assert!(filed[0].get("text_snapshot").is_none());
    assert!(filed[0].get("target_user_id").is_none());
    assert!(!bob.to_string().contains("Exported words"));
}

/// Deleting the author's account clears the text of every report of their texts; the reports
/// stay, gone, for the statistics.
#[sqlx::test]
async fn deleting_the_authors_account_clears_the_snapshots(db: PgPool) {
    let w = world(db).await;
    let post = w.diary(&w.ada, json!({"body": "Words of ada"})).await;
    let quote = w.quote(&w.ada, "Quoted by ada", "audience").await;
    w.take(&w.ada, "Take of ada", "audience").await;
    let bob_post = w.diary(&w.bob, json!({"body": "Words of bob"})).await;
    for path in [post_path(post), quote_path(quote), w.take_path("ada")] {
        id_of(&report(&w.dan, &path, json!({"reason": "other"})).await);
    }
    id_of(&report(&w.dan, &post_path(bob_post), json!({"reason": "other"})).await);
    let decided = id_of(&report(&w.bob, &post_path(post), json!({"reason": "offensive"})).await);
    resolve(&w.root, decided, "dismiss")
        .await
        .assert_status(200);

    w.ada
        .post(
            &format!("/api/clubs/{}/transfer", w.club.slug),
            json!({"username": "bob"}),
        )
        .await
        .assert_status(200);
    w.ada
        .delete_json("/api/auth/me", json!({"password": PASSWORD}))
        .await
        .assert_status(204);
    assert_eq!(
        w.app.scalar_i64("SELECT count(*) FROM reports").await,
        5,
        "the reports stay"
    );
    assert_eq!(
        w.app
            .scalar_i64("SELECT count(*) FROM reports WHERE text_snapshot IS NOT NULL")
            .await,
        1,
        "only bob's"
    );
    // The dismissal closed dan's report of the same post too.
    let open = w.open_reports().await;
    assert_eq!(open.len(), 3);
    for item in &open {
        if item["text_snapshot"] == "Words of bob" {
            continue;
        }
        assert_eq!(item["text_snapshot"], Value::Null, "{item}");
        assert_eq!(item["current_text"], Value::Null);
        assert_eq!(item["target_user"], Value::Null);
        assert_eq!(item["gone"], true);
    }
}

/// Text reports share the daily limit with every other report, and two reports of one text at
/// once by one reader are one report.
#[sqlx::test]
async fn text_reports_share_the_limits_and_take_turns(db: PgPool) {
    let w = world(db).await;
    let mut quotes = Vec::new();
    for n in 0..21 {
        quotes.push(w.quote(&w.ada, &format!("Line {n}"), "audience").await);
    }
    let first = quote_path(quotes[0]);
    let (one, two) = tokio::join!(
        report(&w.bob, &first, json!({"reason": "other"})),
        report(&w.bob, &first, json!({"reason": "other"})),
    );
    assert_eq!(id_of(&one), id_of(&two));
    for quote in &quotes[1..20] {
        id_of(&report(&w.bob, &quote_path(*quote), json!({"reason": "other"})).await);
    }
    report(&w.bob, &quote_path(quotes[20]), json!({"reason": "other"}))
        .await
        .assert_detail(429, "Slow down a little. Try again later.");
    // A repeat still answers.
    id_of(&report(&w.bob, &quote_path(quotes[3]), json!({"reason": "other"})).await);
    assert_eq!(
        w.app
            .scalar_i64("SELECT count(*) FROM reports WHERE target_kind = 'quote'")
            .await,
        20
    );
}

/// Waits until `n` sessions wait for a lock.
async fn until_lock_waits(db: &PgPool, n: i64) {
    for _ in 0..500 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        if waiting >= n {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("fewer than {n} sessions waited for a lock");
}

/// The lock order (docs/phase3/notes/texts.md): a report locks the author's row
/// (`FOR KEY SHARE`) before the text's row, as the author's account deletion does (the user row
/// `FOR UPDATE`, then the quotes through its cascade). The report's row names the author
/// (`target_user_id`), so its foreign key would take that lock anyway; taken only there, after
/// the quote, a report that held the quote waited for the deletion's user row while the deletion
/// waited for the quote, and Postgres aborted one of them (40P01, a 500). Here the deletion holds
/// Ada's row and waits for her quote, which another transaction shares, when Bob's report starts:
/// the report waits for the deletion and is 404, and the deletion goes through.
#[sqlx::test]
async fn a_report_racing_the_authors_account_deletion_does_not_deadlock(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let club = app.club(&bob, "Readers").await;
    app.join(&club, &ada, "member").await;
    let circe = app.work("Circe").await;
    let res = ada
        .post(
            "/api/quotes",
            json!({"work_id": circe.id, "body": "A line"}),
        )
        .await;
    let quote = id_of(&res);

    let mut sharer = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM quotes WHERE id = $1 FOR SHARE")
        .bind(quote)
        .execute(&mut *sharer)
        .await
        .unwrap();
    let deleting = ada.delete_json("/api/auth/me", json!({"password": PASSWORD}));
    let path = quote_path(quote);
    let filing = async {
        until_lock_waits(&app.db, 1).await;
        report(&bob, &path, json!({"reason": "offensive"})).await
    };
    let release = async {
        until_lock_waits(&app.db, 2).await;
        sharer.commit().await.unwrap();
    };
    let (deleted, filed, ()) = tokio::join!(deleting, filing, release);
    deleted.assert_status(204);
    filed.assert_detail(404, "No quote with that id");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 0);
}
