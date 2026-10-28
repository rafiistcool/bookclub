//! Diary entries: a work's thread, writing entries and replies, edits, deletes and tombstones,
//! reactions and the diary feed (endpoint-map §8, docs/phase1/spec/diary.md §1.1–§1.6). The
//! parity tests of `test_diary.py` run between co-members of one club, the multi-club form of
//! the legacy "everyone is in the club".

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const HEART: &str = "\u{2764}\u{fe0f}";
const THUMBS: &str = "\u{1f44d}";
const FIRE: &str = "\u{1f525}";

fn thread(work: &TestWork) -> String {
    format!("/api/books/works/{}/posts", work.id)
}

/// `ada` and `grace`, co-members of one club (each other's audience).
async fn ada_and_grace(app: &TestApp) -> (TestClient, TestClient, TestClub) {
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    (ada, grace, club)
}

async fn write(client: &TestClient, work: &TestWork, body: Value) -> Value {
    let res = client.post(&thread(work), body).await;
    res.assert_status(201);
    res.json()
}

/// Puts the work on the client's shelf and returns the entry id.
async fn shelve(client: &TestClient, work: &TestWork, extra: Value) -> i64 {
    let mut body = json!({ "work_id": work.id });
    if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
    let res = client.post("/api/shelf", body).await;
    res.assert_status(201);
    res.json()["id"].as_i64().unwrap()
}

async fn patch_shelf(client: &TestClient, entry: i64, body: Value) {
    client
        .patch(&format!("/api/shelf/{entry}"), body)
        .await
        .assert_status(200);
}

fn bodies(items: &Value) -> Vec<String> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["body"].as_str().unwrap().to_owned())
        .collect()
}

fn feed_bodies(feed: &Value) -> Vec<String> {
    feed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["entry"]["body"].as_str().unwrap().to_owned())
        .collect()
}

/// The feed items keyed by their book's title.
fn by_title(feed: &Value) -> std::collections::HashMap<String, Value> {
    feed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["book"]["title"].as_str().unwrap().to_owned(),
                item.clone(),
            )
        })
        .collect()
}

/// parity: test_diary.py::test_requires_auth_and_valid_work_id
#[sqlx::test]
async fn requires_auth_and_a_valid_work_id(db: PgPool) {
    let app = TestApp::new(db).await;
    let circe = app.work("Circe").await;
    app.anon()
        .get(&thread(&circe))
        .await
        .assert_detail(401, "Not signed in");
    app.anon()
        .post(&thread(&circe), json!({"body": ""}))
        .await
        .assert_detail(401, "Not signed in");
    let ada = app.user("ada").await;
    ada.get("/api/books/works/nope/posts")
        .await
        .assert_detail(404, "No such book.");
    ada.post("/api/books/works/nope/posts", json!({"body": "x"}))
        .await
        .assert_detail(404, "No such book.");
    // Body validation comes before the work check (spec R4).
    ada.post("/api/books/works/nope/posts", json!({"body": " "}))
        .await
        .assert_detail(400, "Write a short take, quote, or meeting note");
}

/// parity: test_diary.py::test_diary_on_an_unknown_book_is_empty_not_missing
///
/// Reasoned change (docs/phase1/work-plan.md, diary 13): a work the catalog has but nobody
/// wrote about is an empty diary; a work id the catalog does not know is 404 `No such book.`.
#[sqlx::test]
async fn a_work_without_entries_has_an_empty_diary(db: PgPool) {
    let app = TestApp::new(db).await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let ada = app.user("ada").await;
    let res = ada.get("/api/books/works/OL1W/posts").await;
    res.assert_status(200);
    assert_eq!(
        res.json(),
        json!({"work_id": circe.id, "my_progress": null, "my_status": null, "items": [],
               "timezone": "UTC"})
    );
    assert_eq!(ada.get(&thread(&circe)).await.json()["work_id"], circe.id);
    for unknown in ["OL99W", "999999", "BCdeadbeef01", "ISBN9780316769488"] {
        ada.get(&format!("/api/books/works/{unknown}/posts"))
            .await
            .assert_detail(404, "No such book.");
    }
    assert!(app.upstream_requests().await.is_empty());
}

/// parity: test_diary.py::test_first_entry_creates_the_book_when_given_its_details
///
/// Reasoned change (endpoint-map §8): the client no longer sends book metadata; an entry needs
/// a catalog work (404 `No such book.` otherwise), and a `book` key is ignored.
#[sqlx::test]
async fn the_first_entry_needs_a_catalog_work(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    ada.post(
        "/api/books/works/OL1W/posts",
        json!({"body": "Loved the opening", "book": {"title": "Circe"}}),
    )
    .await
    .assert_detail(404, "No such book.");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM works").await, 0);

    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let entry = write(
        &ada,
        &circe,
        json!({"body": "  Loved the opening  ", "book": {"title": "CHANGED"}}),
    )
    .await;
    assert_eq!(entry["author"], "ada");
    assert_eq!(entry["mine"], true);
    assert_eq!(entry["body"], "Loved the opening");
    assert_eq!(entry["progress_at"], Value::Null);
    assert_eq!(entry["status_at"], Value::Null);
    assert_eq!(entry["visibility"], "audience");
    assert_eq!(entry["club"], Value::Null);
    assert_eq!(entry["pick_id"], Value::Null);
    assert_eq!(entry["deleted"], false);
    assert_eq!(entry["edited"], false);
    assert_eq!(entry["replies"], json!([]));
    assert_eq!(entry["reactions"], json!([]));
    assert!(!entry["created_label"].as_str().unwrap().is_empty());
    assert!(entry["created_at"].as_str().unwrap().ends_with('Z'));

    let diary = ada.get(&thread(&circe)).await.json();
    assert_eq!(bodies(&diary["items"]), ["Loved the opening"]);
    // Writing about a book does not shelve it, and the work keeps its title.
    assert_eq!(ada.get("/api/shelf").await.json()["items"], json!([]));
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM works WHERE title = 'Circe'")
            .await,
        1
    );
}

/// parity: test_diary.py::test_entry_snapshots_where_the_author_was
#[sqlx::test]
async fn an_entry_snapshots_where_the_author_was(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = app.work("Circe").await;
    let shelf = shelve(&ada, &circe, json!({"status": "currently_reading"})).await;
    patch_shelf(&ada, shelf, json!({"progress": 40})).await;
    let reading = write(
        &ada,
        &circe,
        json!({"body": "Halfway-ish", "spoiler_upto": 40}),
    )
    .await;
    assert_eq!(reading["progress_at"], 40);
    assert_eq!(reading["status_at"], "currently_reading");
    assert_eq!(reading["spoiler_upto"], 40);
    assert_eq!(reading["author_rating"], Value::Null);

    patch_shelf(&ada, shelf, json!({"status": "finished", "rating": 4})).await;
    let done = write(&ada, &circe, json!({"body": "That ending."})).await;
    assert_eq!(done["progress_at"], 100);
    assert_eq!(done["status_at"], "finished");
    assert_eq!(done["author_rating"], 4);

    let diary = ada.get(&thread(&circe)).await.json();
    let items = diary["items"].as_array().unwrap();
    // The old entry keeps its position, but the author's stars are live.
    assert_eq!(items[0]["progress_at"], 40);
    assert_eq!(items[0]["author_rating"], 4);
    assert_eq!(items[1]["progress_at"], 100);
    assert_eq!(diary["my_progress"], 100);
    assert_eq!(diary["my_status"], "finished");

    // A DNF hides the rating again (only finished authors show one).
    patch_shelf(&ada, shelf, json!({"status": "did_not_finish"})).await;
    let diary = ada.get(&thread(&circe)).await.json();
    assert_eq!(diary["items"][0]["author_rating"], Value::Null);
    assert_eq!(diary["items"][0]["status_at"], "currently_reading");
}

/// parity: test_diary.py::test_viewer_position_drives_the_spoiler_shield_fields
#[sqlx::test]
async fn the_viewer_position_drives_the_spoiler_shield_fields(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, _) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    shelve(&ada, &circe, json!({"status": "finished", "rating": 5})).await;
    write(
        &ada,
        &circe,
        json!({"body": "Ending spoiler", "spoiler_upto": 100}),
    )
    .await;
    let shelf = shelve(&grace, &circe, json!({"status": "currently_reading"})).await;
    patch_shelf(&grace, shelf, json!({"progress": 25})).await;
    let diary = grace.get(&thread(&circe)).await.json();
    assert_eq!(diary["my_progress"], 25);
    assert_eq!(diary["my_status"], "currently_reading");
    assert_eq!(diary["items"][0]["spoiler_upto"], 100);
    assert_eq!(diary["items"][0]["mine"], false);
    // The server never hides bodies; the shield is the client's.
    assert_eq!(diary["items"][0]["body"], "Ending spoiler");
    assert_eq!(diary["items"][0]["author_rating"], 5);
}

/// parity: test_diary.py::test_replies_are_one_level_and_stay_on_the_book
#[sqlx::test]
async fn replies_are_one_level_and_stay_on_the_work(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, _) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    let achilles = app.work("The Song of Achilles").await;
    let root = write(&ada, &circe, json!({"body": "Thoughts?"})).await;
    let reply = grace
        .post(
            &thread(&circe),
            json!({"body": "Yes!", "parent_id": root["id"]}),
        )
        .await;
    reply.assert_status(201);
    assert_eq!(reply.json()["parent_id"], root["id"]);

    grace
        .post(
            &thread(&circe),
            json!({"body": "Deeper", "parent_id": reply.json()["id"]}),
        )
        .await
        .assert_detail(400, "Reply to the entry itself, not to a reply");
    grace
        .post(
            &thread(&achilles),
            json!({"body": "Wrong book", "parent_id": root["id"]}),
        )
        .await
        .assert_detail(400, "That entry is not on this book");
    grace
        .post(
            &thread(&circe),
            json!({"body": "Nobody", "parent_id": 999_999}),
        )
        .await
        .assert_detail(400, "That entry is not on this book");
    // A reply has no visibility of its own.
    grace
        .post(
            &thread(&circe),
            json!({"body": "Secret", "parent_id": root["id"], "visibility": "only_me"}),
        )
        .await
        .assert_detail(400, "Replies and club posts cannot be private");

    let diary = grace.get(&thread(&circe)).await.json();
    let items = diary["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(bodies(&items[0]["replies"]), ["Yes!"]);
    assert_eq!(items[0]["replies"][0]["author"], "grace");
    assert_eq!(items[0]["replies"][0]["replies"], json!([]));
}

/// parity: test_diary.py::test_only_the_author_edits_or_deletes
#[sqlx::test]
async fn only_the_author_edits_or_deletes(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, _) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    let entry = write(&ada, &circe, json!({"body": "Draft"})).await;
    let path = format!("/api/posts/{}", entry["id"]);
    grace
        .patch(&path, json!({"body": "Hijack"}))
        .await
        .assert_detail(403, "You can only edit your own entries");
    // Authorship is checked before "Nothing to update" (spec R6).
    grace
        .patch(&path, json!({}))
        .await
        .assert_detail(403, "You can only edit your own entries");
    grace
        .delete(&path)
        .await
        .assert_detail(403, "You can only delete your own entries");

    ada.patch(&path, json!({}))
        .await
        .assert_detail(400, "Nothing to update");
    ada.patch(&path, json!({"body": null}))
        .await
        .assert_detail(400, "Nothing to update");
    ada.patch(&path, json!({"body": "  "}))
        .await
        .assert_detail(400, "Write a short take, quote, or meeting note");
    ada.patch(&path, json!({"spoiler_upto": 101}))
        .await
        .assert_detail(400, "Input should be less than or equal to 100");
    let edited = ada
        .patch(&path, json!({"body": "Final", "spoiler_upto": 60}))
        .await;
    edited.assert_status(200);
    let edited = edited.json();
    assert_eq!(edited["body"], "Final");
    assert_eq!(edited["spoiler_upto"], 60);
    assert_eq!(edited["edited"], true);

    let cleared = ada.patch(&path, json!({"spoiler_upto": null})).await.json();
    assert_eq!(cleared["spoiler_upto"], Value::Null);
    assert_eq!(cleared["body"], "Final");

    ada.delete(&path).await.assert_status(204);
    assert_eq!(ada.get(&thread(&circe)).await.json()["items"], json!([]));
    assert_eq!(app.scalar_i64("SELECT count(*) FROM posts").await, 0);
    ada.delete(&path)
        .await
        .assert_detail(404, "No entry with that id");
    ada.patch(&path, json!({"body": "Back"}))
        .await
        .assert_detail(404, "No entry with that id");
}

/// Edits bump `updated_at` even when nothing changed (spec R8); reactions never do. An entry
/// can turn `only_me` and back; a reply cannot.
#[sqlx::test]
async fn edits_mark_the_entry_and_change_its_visibility(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, _) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    let entry = write(&ada, &circe, json!({"body": "Same"})).await;
    let path = format!("/api/posts/{}", entry["id"]);
    let reacted = grace
        .post(&format!("{path}/reactions"), json!({"emoji": HEART}))
        .await
        .json();
    assert_eq!(reacted["edited"], false);
    let same = ada.patch(&path, json!({"body": "Same"})).await.json();
    assert_eq!(same["edited"], true);

    let private = ada.patch(&path, json!({"visibility": "only_me"})).await;
    private.assert_status(200);
    assert_eq!(private.json()["visibility"], "only_me");
    assert_eq!(grace.get(&thread(&circe)).await.json()["items"], json!([]));
    grace
        .post(&format!("{path}/reactions"), json!({"emoji": HEART}))
        .await
        .assert_detail(404, "No entry with that id");
    ada.patch(&path, json!({"visibility": "audience"}))
        .await
        .assert_status(200);
    let reply = write(
        &grace,
        &circe,
        json!({"body": "Hello", "parent_id": entry["id"]}),
    )
    .await;
    grace
        .patch(
            &format!("/api/posts/{}", reply["id"]),
            json!({"visibility": "only_me"}),
        )
        .await
        .assert_detail(400, "Replies and club posts cannot be private");
    // "audience" on a reply is harmless, and no edit.
    let reply = grace
        .patch(
            &format!("/api/posts/{}", reply["id"]),
            json!({"visibility": "audience"}),
        )
        .await;
    reply.assert_status(200);
    assert_eq!(reply.json()["edited"], false);
}

/// Changing only who sees an entry is no edit: `updated_at` stays as it is, so the entry does
/// not show "edited" to its readers (fe.md, "Server issues found by the smoke test"). A body or
/// spoiler change still is one, with or without a visibility change beside it (R8).
#[sqlx::test]
async fn a_visibility_change_alone_is_no_edit(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, _) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    let entry = write(&ada, &circe, json!({"body": "Mine"})).await;
    let path = format!("/api/posts/{}", entry["id"]);
    let updated_at = async || -> Option<time::OffsetDateTime> {
        sqlx::query_scalar("SELECT updated_at FROM posts WHERE id = $1")
            .bind(entry["id"].as_i64().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap()
    };

    for visibility in ["only_me", "audience"] {
        let res = ada.patch(&path, json!({"visibility": visibility})).await;
        res.assert_status(200);
        assert_eq!(res.json()["visibility"], visibility);
        assert_eq!(res.json()["edited"], false, "{visibility}");
    }
    assert_eq!(updated_at().await, None);
    let seen = grace.get(&thread(&circe)).await.json();
    assert_eq!(seen["items"][0]["edited"], false);

    let res = ada.patch(&path, json!({"spoiler_upto": 40})).await;
    assert_eq!(res.json()["edited"], true);
    let edited_at = updated_at().await.expect("a spoiler change is an edit");
    let res = ada.patch(&path, json!({"visibility": "only_me"})).await;
    assert_eq!(res.json()["edited"], true);
    assert_eq!(updated_at().await, Some(edited_at));
    let res = ada
        .patch(&path, json!({"body": "Mine", "visibility": "audience"}))
        .await;
    assert_eq!(res.json()["visibility"], "audience");
    assert_eq!(res.json()["edited"], true);
    assert!(updated_at().await > Some(edited_at));
}

/// parity: test_diary.py::test_deleting_an_entry_with_replies_leaves_a_tombstone
#[sqlx::test]
async fn deleting_an_entry_with_replies_leaves_a_tombstone(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, _) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    shelve(
        &ada,
        &circe,
        json!({"status": "currently_reading", "progress": 30}),
    )
    .await;
    let root = write(&ada, &circe, json!({"body": "Root", "spoiler_upto": 50})).await;
    let path = format!("/api/posts/{}", root["id"]);
    ada.post(&format!("{path}/reactions"), json!({"emoji": HEART}))
        .await
        .assert_status(200);
    let reply = write(
        &grace,
        &circe,
        json!({"body": "Reply", "parent_id": root["id"]}),
    )
    .await;
    // A root's PATCH and reaction answers carry its replies (spec R9); a reply's carry none.
    let patched = ada.patch(&path, json!({"body": "Root"})).await;
    patched.assert_status(200);
    assert_eq!(bodies(&patched.json()["replies"]), ["Reply"]);
    let reacted = ada
        .post(&format!("{path}/reactions"), json!({"emoji": FIRE}))
        .await;
    reacted.assert_status(200);
    assert_eq!(bodies(&reacted.json()["replies"]), ["Reply"]);
    let reply_path = format!("/api/posts/{}", reply["id"]);
    let patched_reply = grace.patch(&reply_path, json!({"body": "Reply"})).await;
    patched_reply.assert_status(200);
    assert_eq!(patched_reply.json()["replies"], json!([]));

    ada.delete(&path).await.assert_status(204);
    let diary = ada.get(&thread(&circe)).await.json();
    let ghost = &diary["items"][0];
    assert_eq!(ghost["deleted"], true);
    assert_eq!(ghost["body"], "");
    assert_eq!(ghost["spoiler_upto"], Value::Null);
    assert_eq!(ghost["reactions"], json!([]));
    assert_eq!(ghost["edited"], false);
    // The tombstone keeps its author and the position snapshot.
    assert_eq!(ghost["author"], "ada");
    assert_eq!(ghost["progress_at"], 30);
    assert_eq!(bodies(&ghost["replies"]), ["Reply"]);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM post_reactions").await,
        0
    );

    // Nobody can bring it back or hang more on it.
    ada.patch(&path, json!({"body": "x"}))
        .await
        .assert_detail(404, "No entry with that id");
    ada.post(&format!("{path}/reactions"), json!({"emoji": THUMBS}))
        .await
        .assert_detail(404, "No entry with that id");
    grace
        .post(
            &thread(&circe),
            json!({"body": "Late", "parent_id": root["id"]}),
        )
        .await
        .assert_detail(400, "That entry was deleted");
    ada.delete(&path)
        .await
        .assert_detail(404, "No entry with that id");
    // Tombstones stay out of the feed; their replies do not, and still name the tombstone's
    // author as `parent_author` (spec R2).
    for viewer in [&ada, &grace] {
        let feed = viewer.get("/api/diary").await.json();
        assert_eq!(feed_bodies(&feed), ["Reply"]);
        assert_eq!(feed["items"][0]["parent_author"], "ada");
    }

    // Deleting a reply is a hard delete; the tombstone stays as an anchor.
    grace.delete(&reply_path).await.assert_status(204);
    let diary = ada.get(&thread(&circe)).await.json();
    assert_eq!(diary["items"][0]["deleted"], true);
    assert_eq!(diary["items"][0]["replies"], json!([]));
}

/// parity: test_diary.py::test_reactions_toggle_and_aggregate
#[sqlx::test]
async fn reactions_toggle_and_aggregate(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, _) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    let entry = write(&ada, &circe, json!({"body": "Root"})).await;
    let url = format!("/api/posts/{}/reactions", entry["id"]);
    for refused in ["\u{1f389}", "\u{2764}", "", "❤️❤️"] {
        ada.post(&url, json!({"emoji": refused}))
            .await
            .assert_detail(400, "Pick one of the club reactions");
    }
    ada.post(&url, json!({}))
        .await
        .assert_detail(400, "Field required");

    let first = ada.post(&url, json!({"emoji": HEART})).await;
    first.assert_status(200);
    assert_eq!(
        first.json()["reactions"],
        json!([{"emoji": HEART, "count": 1, "mine": true, "users": ["ada"]}])
    );

    let second = grace.post(&url, json!({"emoji": HEART})).await.json();
    assert_eq!(second["reactions"][0]["count"], 2);
    assert_eq!(second["reactions"][0]["mine"], true);
    assert_eq!(second["reactions"][0]["users"], json!(["ada", "grace"]));

    let untoggled = grace.post(&url, json!({"emoji": HEART})).await.json();
    assert_eq!(
        untoggled["reactions"],
        json!([{"emoji": HEART, "count": 1, "mine": false, "users": ["ada"]}])
    );
    // Surrounding whitespace is ignored.
    let fire = grace
        .post(&url, json!({"emoji": format!(" {FIRE} ")}))
        .await
        .json();
    assert_eq!(fire["reactions"][1]["emoji"], FIRE);
    // Reactions on diary entries record no event (parity).
    assert_eq!(app.scalar_i64("SELECT count(*) FROM events").await, 0);
}

/// parity: test_diary.py::test_feed_exposes_viewer_position_for_spoiler_shield
#[sqlx::test]
async fn the_feed_exposes_the_viewer_position_for_the_spoiler_shield(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, _) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    let achilles = app.work("The Song of Achilles").await;
    let shelf = shelve(&ada, &circe, json!({"status": "currently_reading"})).await;
    patch_shelf(&ada, shelf, json!({"progress": 80})).await;
    let spoiler = "The coconut pillow on p. 237 is a tell.";
    write(&ada, &circe, json!({"body": spoiler, "spoiler_upto": 70})).await;
    write(&ada, &achilles, json!({"body": "Plain note"})).await;

    let own = by_title(&ada.get("/api/diary").await.json());
    assert_eq!(own["Circe"]["my_progress"], 80);
    assert_eq!(own["Circe"]["my_status"], "currently_reading");
    assert_eq!(own["Circe"]["entry"]["mine"], true);
    assert_eq!(own["Circe"]["entry"]["body"], spoiler);
    assert_eq!(own["The Song of Achilles"]["my_progress"], Value::Null);
    assert_eq!(own["The Song of Achilles"]["my_status"], Value::Null);

    let unshelved = by_title(&grace.get("/api/diary").await.json());
    assert_eq!(unshelved["Circe"]["my_progress"], Value::Null);
    assert_eq!(unshelved["Circe"]["my_status"], Value::Null);
    assert_eq!(unshelved["Circe"]["entry"]["mine"], false);
    assert_eq!(unshelved["Circe"]["entry"]["spoiler_upto"], 70);

    let shelf = shelve(&grace, &circe, json!({"status": "currently_reading"})).await;
    patch_shelf(&grace, shelf, json!({"progress": 25})).await;
    let behind = by_title(&grace.get("/api/diary").await.json());
    assert_eq!(behind["Circe"]["my_progress"], 25);
    assert_eq!(behind["Circe"]["my_status"], "currently_reading");
    assert_eq!(behind["The Song of Achilles"]["my_progress"], Value::Null);

    patch_shelf(&grace, shelf, json!({"status": "finished", "rating": 4})).await;
    let done = by_title(&grace.get("/api/diary").await.json());
    assert_eq!(done["Circe"]["my_progress"], 100);
    assert_eq!(done["Circe"]["my_status"], "finished");

    patch_shelf(&grace, shelf, json!({"status": "did_not_finish"})).await;
    let dnf = by_title(&grace.get("/api/diary").await.json());
    assert_eq!(dnf["Circe"]["my_progress"], Value::Null);
    assert_eq!(dnf["Circe"]["my_status"], "did_not_finish");
}

/// parity: test_diary.py::test_feed_is_newest_first_across_books_and_pages
#[sqlx::test]
async fn the_feed_is_newest_first_across_works_and_pages(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, _) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    let achilles = app.work("The Song of Achilles").await;
    let root = write(&ada, &circe, json!({"body": "Circe 1"})).await;
    write(&ada, &achilles, json!({"body": "Achilles 1"})).await;
    write(
        &grace,
        &circe,
        json!({"body": "Circe reply", "parent_id": root["id"]}),
    )
    .await;

    let page = grace.get("/api/diary?limit=2").await.json();
    assert_eq!(feed_bodies(&page), ["Circe reply", "Achilles 1"]);
    let titles: Vec<&str> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["book"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Circe", "The Song of Achilles"]);
    assert_eq!(page["items"][0]["parent_author"], "ada");
    assert_eq!(page["items"][1]["parent_author"], Value::Null);
    assert_eq!(page["items"][0]["entry"]["replies"], json!([]));
    assert_eq!(page["items"][0]["club"], Value::Null);
    assert_eq!(page["has_more"], true);
    assert_eq!(page["timezone"], "UTC");

    let last = page["items"][1]["entry"]["id"].as_i64().unwrap();
    let rest = grace
        .get(&format!("/api/diary?limit=2&before={last}"))
        .await
        .json();
    assert_eq!(feed_bodies(&rest), ["Circe 1"]);
    assert_eq!(rest["has_more"], false);
    // Replies stay in the feed with no replies of their own; the default limit is 10.
    assert_eq!(feed_bodies(&grace.get("/api/diary").await.json()).len(), 3);

    for (query, detail) in [
        ("limit=0", "Input should be greater than or equal to 1"),
        ("limit=51", "Input should be less than or equal to 50"),
        ("before=0", "Input should be greater than or equal to 1"),
        (
            "before=x",
            "Input should be a valid integer, unable to parse string as an integer",
        ),
    ] {
        grace
            .get(&format!("/api/diary?{query}"))
            .await
            .assert_detail(400, detail);
    }
}

/// Threads order roots and replies by `created_at, id`, the feed pages by id (endpoint-map §8,
/// spec diary R1): an imported post can be older than its id says. Moving the later root and
/// the later reply back in time reorders both threads, not the feed.
#[sqlx::test]
async fn threads_follow_the_time_and_the_feed_the_id(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, club) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    sqlx::query("INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2)")
        .bind(club.id)
        .bind(circe.id)
        .execute(&app.db)
        .await
        .unwrap();
    let pick_thread = format!("/api/clubs/{}/pick/posts", club.slug);
    let post = |client: &TestClient, body: Value| {
        let (client, path) = (client.clone(), pick_thread.clone());
        async move {
            let res = client.post(&path, body).await;
            res.assert_status(201);
            res.json()["id"].as_i64().unwrap()
        }
    };
    let first = post(&ada, json!({"body": "First root"})).await;
    let second = post(&grace, json!({"body": "Second root"})).await;
    let first_reply = post(&grace, json!({"body": "First reply", "parent_id": first})).await;
    let second_reply = post(&ada, json!({"body": "Second reply", "parent_id": first})).await;
    assert!(first < second && second < first_reply && first_reply < second_reply);
    app.exec(&format!(
        "UPDATE posts SET created_at = now() - interval '1 day'
         WHERE id IN ({second}, {second_reply})"
    ))
    .await;

    for path in [thread(&circe), pick_thread.clone()] {
        let items = grace.get(&path).await.json()["items"].clone();
        assert_eq!(bodies(&items), ["Second root", "First root"], "{path}");
        assert_eq!(
            bodies(&items[1]["replies"]),
            ["Second reply", "First reply"],
            "{path}"
        );
    }
    let page = grace.get("/api/diary?limit=2").await.json();
    assert_eq!(feed_bodies(&page), ["Second reply", "First reply"]);
    assert_eq!(page["has_more"], true);
    let rest = grace
        .get(&format!("/api/diary?limit=2&before={first_reply}"))
        .await
        .json();
    assert_eq!(feed_bodies(&rest), ["Second root", "First root"]);
    assert_eq!(rest["has_more"], false);
}

/// parity: test_diary.py::test_custom_book_has_a_diary_without_open_library
///
/// A user-added work (the legacy `BC…` custom book) has a diary like any other, also under
/// its legacy key, and nothing goes outbound.
#[sqlx::test]
async fn a_user_added_work_has_a_diary_without_open_library(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let zine = app.work("Kitchen Zine").await;
    sqlx::query(
        "INSERT INTO legacy_work_keys (legacy_key, legacy_book_id, work_id, match)
         VALUES ('/works/BCdeadbeef01', 7, $1, 'user')",
    )
    .bind(zine.id)
    .execute(&app.db)
    .await
    .unwrap();
    let empty = ada.get("/api/books/works/BCdeadbeef01/posts").await.json();
    assert_eq!(empty["work_id"], zine.id);
    assert_eq!(empty["items"], json!([]));
    let posted = ada
        .post(
            "/api/books/works/BCdeadbeef01/posts",
            json!({"body": "First issue is out."}),
        )
        .await;
    posted.assert_status(201);
    assert_eq!(
        bodies(&ada.get(&thread(&zine)).await.json()["items"]),
        ["First issue is out."]
    );
    assert!(app.upstream_requests().await.is_empty());
}

/// A diary entry's label is in the viewer's zone; the thread says which zone that is.
#[sqlx::test]
async fn diary_labels_use_the_viewers_zone(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, _) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    let entry = write(&ada, &circe, json!({"body": "Hello"})).await;
    assert!(entry["created_label"].as_str().unwrap().ends_with(" UTC"));
    app.exec(&format!(
        "UPDATE users SET timezone = 'America/New_York' WHERE id = {}",
        grace.id()
    ))
    .await;
    let diary = grace.get(&thread(&circe)).await.json();
    assert_eq!(diary["timezone"], "America/New_York");
    let label = diary["items"][0]["created_label"].as_str().unwrap();
    assert!(
        label.ends_with(" EST") || label.ends_with(" EDT"),
        "{label}"
    );
    assert_eq!(
        grace.get("/api/diary").await.json()["timezone"],
        "America/New_York"
    );
}

/// A merged-away work id finds the thread of the work it was merged into.
#[sqlx::test]
async fn a_merged_work_answers_with_its_target(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = app.work("Circe").await;
    let old = app.work("Circe (old)").await;
    write(&ada, &circe, json!({"body": "On the real one"})).await;
    app.exec(&format!(
        "UPDATE works SET merged_into_id = {} WHERE id = {}",
        circe.id, old.id
    ))
    .await;
    let diary = ada.get(&thread(&old)).await.json();
    assert_eq!(diary["work_id"], circe.id);
    assert_eq!(bodies(&diary["items"]), ["On the real one"]);
    let written = write(&ada, &old, json!({"body": "Via the old id"})).await;
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT work_id FROM posts WHERE id = {}",
            written["id"]
        ))
        .await,
        circe.id
    );
}

/// `?club=` narrows the feed to one club: its posts and the diary entries of its current
/// members. A club the caller is not in is 404.
#[sqlx::test]
async fn the_club_feed_shows_one_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, club) = ada_and_grace(&app).await;
    let tom = app.user("tom").await;
    let side = app.club(&tom, "Side").await;
    app.join(&side, &grace, "member").await;
    let circe = app.work("Circe").await;
    let pick: i64 = sqlx::query_scalar(
        "INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(club.id)
    .bind(circe.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    let side_pick: i64 = sqlx::query_scalar(
        "INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(side.id)
    .bind(circe.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    write(&ada, &circe, json!({"body": "Ada's diary"})).await;
    write(&tom, &circe, json!({"body": "Tom's diary"})).await;
    ada.post(
        &format!("/api/clubs/{}/pick/posts", club.slug),
        json!({"body": "Readers' pick post"}),
    )
    .await
    .assert_status(201);
    tom.post(
        &format!("/api/clubs/{}/pick/posts", side.slug),
        json!({"body": "Side pick post"}),
    )
    .await
    .assert_status(201);
    assert!(pick > 0 && side_pick > 0);

    let all = grace.get("/api/diary").await.json();
    assert_eq!(
        feed_bodies(&all),
        [
            "Side pick post",
            "Readers' pick post",
            "Tom's diary",
            "Ada's diary"
        ]
    );
    let readers = grace
        .get(&format!("/api/diary?club={}", club.slug))
        .await
        .json();
    assert_eq!(feed_bodies(&readers), ["Readers' pick post", "Ada's diary"]);
    assert_eq!(
        readers["items"][0]["club"],
        json!({"slug": club.slug, "name": "Readers"})
    );
    assert_eq!(
        readers["items"][0]["entry"]["club"],
        readers["items"][0]["club"]
    );
    // Empty means no filter; a club the caller is not in is 404.
    assert_eq!(
        feed_bodies(&grace.get("/api/diary?club=").await.json()).len(),
        4
    );
    ada.get(&format!("/api/diary?club={}", side.slug))
        .await
        .assert_detail(404, "No such club");
    ada.get("/api/diary?club=nope")
        .await
        .assert_detail(404, "No such club");
}

/// `?mine=true` keeps the caller's own diary writing: their entries (private ones too) and their
/// replies to diary entries, newest first and paged like the whole feed. Club posts and replies
/// to them, other readers' entries and tombstones stay out; it combines with `?club=`.
#[sqlx::test]
async fn the_mine_filter_keeps_the_callers_diary_writing(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, club) = ada_and_grace(&app).await;
    let circe = app.work("Circe").await;
    let achilles = app.work("The Song of Achilles").await;
    sqlx::query("INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2)")
        .bind(club.id)
        .bind(circe.id)
        .execute(&app.db)
        .await
        .unwrap();
    let graces = write(&grace, &circe, json!({"body": "Grace's entry"})).await;
    write(&ada, &circe, json!({"body": "Ada's entry"})).await;
    write(
        &ada,
        &achilles,
        json!({"body": "Ada's private entry", "visibility": "only_me"}),
    )
    .await;
    write(
        &ada,
        &circe,
        json!({"body": "Ada's reply", "parent_id": graces["id"]}),
    )
    .await;
    let pick_post = ada
        .post(
            &format!("/api/clubs/{}/pick/posts", club.slug),
            json!({"body": "Ada's pick post"}),
        )
        .await;
    pick_post.assert_status(201);
    write(
        &ada,
        &circe,
        json!({"body": "Ada's club reply", "parent_id": pick_post.json()["id"]}),
    )
    .await;
    let gone = write(&ada, &achilles, json!({"body": "Ada's deleted entry"})).await;
    ada.delete(&format!("/api/posts/{}", gone["id"]))
        .await
        .assert_status(204);

    let mine = ada.get("/api/diary?mine=true").await.json();
    assert_eq!(
        feed_bodies(&mine),
        ["Ada's reply", "Ada's private entry", "Ada's entry"]
    );
    assert_eq!(mine["items"][0]["parent_author"], "grace");
    let page = ada.get("/api/diary?mine=1&limit=2").await.json();
    assert_eq!(feed_bodies(&page), ["Ada's reply", "Ada's private entry"]);
    assert_eq!(page["has_more"], true);
    let last = page["items"][1]["entry"]["id"].as_i64().unwrap();
    let rest = ada
        .get(&format!("/api/diary?mine=true&limit=2&before={last}"))
        .await
        .json();
    assert_eq!(feed_bodies(&rest), ["Ada's entry"]);
    assert_eq!(rest["has_more"], false);
    assert_eq!(
        feed_bodies(
            &ada.get(&format!("/api/diary?mine=true&club={}", club.slug))
                .await
                .json()
        ),
        ["Ada's reply", "Ada's private entry", "Ada's entry"]
    );
    assert_eq!(
        feed_bodies(&grace.get("/api/diary?mine=true").await.json()),
        ["Grace's entry"]
    );
    // `false` is the whole feed; anything that is not a boolean is refused.
    assert_eq!(
        feed_bodies(&ada.get("/api/diary?mine=false").await.json()).len(),
        6
    );
    ada.get("/api/diary?mine=maybe").await.assert_detail(
        400,
        "Input should be a valid boolean, unable to interpret input",
    );
}

/// parity: test_diary.py::test_club_rating_averages_finished_readers
///
/// The book page (CAT's `GET /api/books/works/{work_id}`) averages the ratings of the
/// viewer's audience who finished the book; a reader without a rating is not counted, and a
/// work nobody rated has no mean. The legacy club is one club of co-members here.
#[sqlx::test]
async fn the_club_rating_averages_finished_readers(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, club) = ada_and_grace(&app).await;
    let tom = app.user("tom").await;
    app.join(&club, &tom, "member").await;
    let circe = app.work("Circe").await;
    shelve(&ada, &circe, json!({"status": "finished", "rating": 5})).await;
    shelve(&grace, &circe, json!({"status": "finished", "rating": 4})).await;
    // Reading with no rating does not count.
    shelve(&tom, &circe, json!({"status": "currently_reading"})).await;

    let body = tom.get(&format!("/api/books/works/{}", circe.id)).await;
    body.assert_status(200);
    assert_eq!(body.json()["club_rating"], 4.5);
    assert_eq!(body.json()["rating_count"], 2);

    let ivy = app.user("ivy").await;
    app.join(&club, &ivy, "member").await;
    let fresh = app.work("The Song of Achilles").await;
    let body = ivy.get(&format!("/api/books/works/{}", fresh.id)).await;
    body.assert_status(200);
    assert_eq!(body.json()["club_rating"], Value::Null);
    assert_eq!(body.json()["rating_count"], 0);
}
