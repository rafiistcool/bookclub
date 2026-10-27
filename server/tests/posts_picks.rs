//! Club pick discussions: the threads under `/api/clubs/{slug}/pick/posts` and
//! `/api/clubs/{slug}/picks/{pick_id}/posts`, edits, moderation, reactions and their events
//! (endpoint-map §8, docs/phase1/spec/club.md §1.5–§1.11). Picks and milestones are inserted
//! with SQL: their routes belong to CLUB.

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const HEART: &str = "\u{2764}\u{fe0f}";
const THUMBS: &str = "\u{1f44d}";
const FIRE: &str = "\u{1f525}";

/// Ends the club's current pick (if any) and makes `work` the current pick.
async fn set_pick(app: &TestApp, club: &TestClub, work: &TestWork) -> i64 {
    app.exec(&format!(
        "UPDATE club_picks SET ended_at = now() WHERE club_id = {} AND ended_at IS NULL",
        club.id
    ))
    .await;
    sqlx::query_scalar("INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2) RETURNING id")
        .bind(club.id)
        .bind(work.id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn milestone(app: &TestApp, club: &TestClub, pick: i64, title: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO milestones (club_id, pick_id, title) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(club.id)
    .bind(pick)
    .bind(title)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// `ada` owns the club `Readers`, `grace` is a member, and Circe is the current pick.
struct Club {
    app: TestApp,
    ada: TestClient,
    grace: TestClient,
    club: TestClub,
    circe: TestWork,
    pick: i64,
}

async fn club_with_pick(db: PgPool) -> Club {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let circe = app.work("Circe").await;
    let pick = set_pick(&app, &club, &circe).await;
    Club {
        app,
        ada,
        grace,
        club,
        circe,
        pick,
    }
}

impl Club {
    fn current(&self) -> String {
        format!("/api/clubs/{}/pick/posts", self.club.slug)
    }

    fn of_pick(&self, pick: i64) -> String {
        format!("/api/clubs/{}/picks/{pick}/posts", self.club.slug)
    }

    async fn post(&self, client: &TestClient, body: Value) -> Value {
        let res = client.post(&self.current(), body).await;
        res.assert_status(201);
        res.json()
    }
}

fn bodies(items: &Value) -> Vec<String> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["body"].as_str().unwrap().to_owned())
        .collect()
}

/// parity: test_pick_posts.py::test_posts_require_auth_and_a_pick
#[sqlx::test]
async fn posts_require_auth_and_a_pick(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let path = format!("/api/clubs/{}/pick/posts", club.slug);
    app.anon()
        .get(&path)
        .await
        .assert_detail(401, "Not signed in");
    app.anon()
        .post(&path, json!({"body": "hello"}))
        .await
        .assert_detail(401, "Not signed in");
    ada.get(&path).await.assert_detail(404, "No club pick yet");
    ada.post(&path, json!({"body": "hello"}))
        .await
        .assert_detail(404, "No club pick yet");
    // Validation runs before the pick check (spec club §1.6).
    ada.post(&path, json!({"body": " "}))
        .await
        .assert_detail(400, "Write a short take, quote, or meeting note");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM posts").await, 0);
}

/// parity: test_pick_posts.py::test_members_can_post_and_read_oldest_first
#[sqlx::test]
async fn members_post_and_read_oldest_first(db: PgPool) {
    let c = club_with_pick(db).await;
    let first = c
        .ada
        .post(&c.current(), json!({"body": "  Opening note  "}))
        .await;
    first.assert_status(201);
    let first = first.json();
    assert_eq!(first["author"], "ada");
    assert_eq!(first["body"], "Opening note");
    assert!(!first["created_label"].as_str().unwrap().is_empty());
    assert_eq!(
        first["club"],
        json!({"slug": c.club.slug, "name": "Readers"})
    );
    assert_eq!(first["pick_id"], c.pick);
    assert_eq!(first["visibility"], "audience");
    c.post(&c.grace, json!({"body": "Grace was here"})).await;

    let thread = c.grace.get(&c.current()).await.json();
    assert_eq!(thread["pick_id"], c.pick);
    assert_eq!(thread["can_post"], true);
    assert_eq!(thread["timezone"], "UTC");
    assert_eq!(bodies(&thread["items"]), ["Opening note", "Grace was here"]);
    let authors: Vec<&str> = thread["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["author"].as_str().unwrap())
        .collect();
    assert_eq!(authors, ["ada", "grace"]);

    let by_id = c.grace.get(&c.of_pick(c.pick)).await.json();
    assert_eq!(bodies(&by_id["items"]), ["Opening note", "Grace was here"]);
    assert_eq!(by_id, thread);
}

/// parity: test_pick_posts.py::test_old_thread_stays_when_pick_changes
#[sqlx::test]
async fn the_old_thread_stays_when_the_pick_changes(db: PgPool) {
    let c = club_with_pick(db).await;
    let old = c.post(&c.ada, json!({"body": "Circe notes"})).await;
    let achilles = c.app.work("The Song of Achilles").await;
    let new_pick = set_pick(&c.app, &c.club, &achilles).await;
    assert_ne!(new_pick, c.pick);

    let current = c.ada.get(&c.current()).await.json();
    assert_eq!(current["pick_id"], new_pick);
    assert_eq!(current["items"], json!([]));
    assert_eq!(current["can_post"], true);

    let archived = c.ada.get(&c.of_pick(c.pick)).await.json();
    assert_eq!(archived["can_post"], false);
    assert_eq!(bodies(&archived["items"]), ["Circe notes"]);

    c.ada
        .post(&c.of_pick(c.pick), json!({"body": "too late"}))
        .await
        .assert_detail(400, "That pick is closed");
    // A closed pick takes no replies either, whichever route they come through.
    c.grace
        .post(
            &format!("/api/books/works/{}/posts", c.circe.id),
            json!({"body": "too late", "parent_id": old["id"]}),
        )
        .await
        .assert_detail(400, "That pick is closed");
    c.grace
        .post(
            &c.current(),
            json!({"body": "wrong pick", "parent_id": old["id"]}),
        )
        .await
        .assert_detail(400, "That entry is not on this book");
    // The closed check comes before the milestone check (spec club §1.8).
    c.ada
        .post(
            &c.of_pick(c.pick),
            json!({"body": "late", "milestone_id": 999_999}),
        )
        .await
        .assert_detail(400, "That pick is closed");
    assert_eq!(c.app.scalar_i64("SELECT count(*) FROM posts").await, 1);
}

/// parity: test_pick_posts.py::test_post_time_uses_club_timezone
#[sqlx::test]
async fn post_times_use_the_club_timezone(db: PgPool) {
    let c = club_with_pick(db).await;
    c.ada
        .patch(
            &format!("/api/clubs/{}", c.club.slug),
            json!({"timezone": "America/New_York"}),
        )
        .await
        .assert_status(200);
    let post = c
        .post(&c.ada, json!({"body": "Hello from the meeting"}))
        .await;
    let label = post["created_label"].as_str().unwrap();
    assert!(label.contains("EDT") || label.contains("EST"), "{label}");
    let thread = c.ada.get(&c.current()).await.json();
    assert_eq!(thread["timezone"], "America/New_York");
    // A club post keeps its club's zone in a viewer's own zone too (the work thread).
    c.app
        .exec(&format!(
            "UPDATE users SET timezone = 'Europe/Berlin' WHERE id = {}",
            c.grace.id()
        ))
        .await;
    let diary = c
        .grace
        .get(&format!("/api/books/works/{}/posts", c.circe.id))
        .await
        .json();
    assert_eq!(diary["timezone"], "Europe/Berlin");
    assert_eq!(diary["items"][0]["created_label"], label);
}

/// parity: test_pick_posts.py::test_invalid_post_body
#[sqlx::test]
async fn invalid_post_bodies(db: PgPool) {
    let c = club_with_pick(db).await;
    c.ada
        .post(&c.current(), json!({"body": "   "}))
        .await
        .assert_detail(400, "Write a short take, quote, or meeting note");
    c.ada
        .post(&c.current(), json!({"body": "x".repeat(1001)}))
        .await
        .assert_detail(400, "Keep it to 1000 characters");
    c.ada
        .post(&c.current(), json!({"body": 5}))
        .await
        .assert_detail(400, "Input should be a valid string");
    // 1000 characters after the strip pass.
    c.post(&c.ada, json!({"body": format!("  {}  ", "ä".repeat(1000))}))
        .await;
}

/// parity: test_notes_and_meeting.py::test_post_carries_spoiler_flag_and_viewer_progress
#[sqlx::test]
async fn a_post_carries_its_spoiler_flag_and_the_viewer_progress(db: PgPool) {
    let c = club_with_pick(db).await;
    let post = c
        .post(
            &c.ada,
            json!({"body": "The pig scene!", "spoiler_upto": 40}),
        )
        .await;
    assert_eq!(post["spoiler_upto"], 40);
    assert_eq!(post["mine"], true);
    assert_eq!(post["edited"], false);
    assert_eq!(post["reactions"], json!([]));

    let thread = c.ada.get(&c.current()).await.json();
    assert_eq!(thread["my_progress"], Value::Null);
    assert_eq!(thread["my_status"], Value::Null);

    let shelf = c
        .ada
        .post(
            "/api/shelf",
            json!({"work_id": c.circe.id, "status": "currently_reading", "progress": 25}),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let thread = c.ada.get(&c.current()).await.json();
    assert_eq!(thread["my_progress"], 25);
    assert_eq!(thread["my_status"], "currently_reading");

    c.ada
        .patch(
            &format!("/api/shelf/{shelf}"),
            json!({"status": "finished", "position": 0}),
        )
        .await
        .assert_status(200);
    let thread = c.ada.get(&c.current()).await.json();
    assert_eq!(thread["my_progress"], 100);
    assert_eq!(thread["my_status"], "finished");
}

/// parity: test_notes_and_meeting.py::test_spoiler_flag_must_be_a_percentage
#[sqlx::test]
async fn the_spoiler_flag_must_be_a_percentage(db: PgPool) {
    let c = club_with_pick(db).await;
    c.ada
        .post(&c.current(), json!({"body": "x", "spoiler_upto": 101}))
        .await
        .assert_detail(400, "Input should be less than or equal to 100");
    c.ada
        .post(&c.current(), json!({"body": "x", "spoiler_upto": -1}))
        .await
        .assert_detail(400, "Input should be greater than or equal to 0");
    // Every message, in field order.
    c.ada
        .post(&c.current(), json!({"body": "", "spoiler_upto": 101}))
        .await
        .assert_detail(
            400,
            "Write a short take, quote, or meeting note; Input should be less than or equal to 100",
        );
}

/// parity: test_notes_and_meeting.py::test_author_can_edit_and_delete_own_note
///
/// The legacy `/api/pick/posts/{post_id}` routes are unified with the diary's
/// `/api/posts/{post_id}` (endpoint-map §8).
#[sqlx::test]
async fn the_author_edits_and_deletes_their_post(db: PgPool) {
    let c = club_with_pick(db).await;
    let post = c.post(&c.ada, json!({"body": "Frist"})).await;
    let path = format!("/api/posts/{}", post["id"]);

    let edited = c.ada.patch(&path, json!({"body": "First"})).await;
    edited.assert_status(200);
    assert_eq!(edited.json()["body"], "First");
    assert_eq!(edited.json()["edited"], true);

    let flagged = c.ada.patch(&path, json!({"spoiler_upto": 60})).await.json();
    assert_eq!(flagged["spoiler_upto"], 60);
    let cleared = c
        .ada
        .patch(&path, json!({"spoiler_upto": null}))
        .await
        .json();
    assert_eq!(cleared["spoiler_upto"], Value::Null);
    c.ada
        .patch(&path, json!({}))
        .await
        .assert_detail(400, "Nothing to update");
    c.ada
        .patch(&path, json!({"visibility": "only_me"}))
        .await
        .assert_detail(400, "Replies and club posts cannot be private");
    // The milestone is not editable; the key is ignored.
    c.ada
        .patch(&path, json!({"milestone_id": 5}))
        .await
        .assert_detail(400, "Nothing to update");

    c.ada.delete(&path).await.assert_status(204);
    assert_eq!(c.ada.get(&c.current()).await.json()["items"], json!([]));
    c.ada
        .delete(&path)
        .await
        .assert_detail(404, "No entry with that id");
}

/// parity: test_notes_and_meeting.py::test_other_members_cannot_edit_or_delete
#[sqlx::test]
async fn other_members_cannot_edit_or_delete(db: PgPool) {
    let c = club_with_pick(db).await;
    let post = c.post(&c.grace, json!({"body": "Grace's note"})).await;
    let path = format!("/api/posts/{}", post["id"]);
    let tom = c.app.user("tom").await;
    c.app.join(&c.club, &tom, "member").await;
    tom.patch(&path, json!({"body": "hijack"}))
        .await
        .assert_detail(403, "You can only edit your own entries");
    tom.delete(&path)
        .await
        .assert_detail(403, "You can only delete your own entries");
    assert_eq!(
        tom.get(&c.current()).await.json()["items"][0]["mine"],
        false
    );
    // The owner moderates (deletes) but never edits someone else's post.
    c.ada
        .patch(&path, json!({"body": "hijack"}))
        .await
        .assert_detail(403, "You can only edit your own entries");
    assert_eq!(
        c.app
            .scalar_i64("SELECT count(*) FROM posts WHERE body = 'Grace''s note'")
            .await,
        1
    );
}

/// parity: test_notes_and_meeting.py::test_reactions_toggle_and_aggregate
#[sqlx::test]
async fn reactions_on_club_posts_toggle_and_record_events(db: PgPool) {
    let c = club_with_pick(db).await;
    let post = c.post(&c.ada, json!({"body": "Loved the ending"})).await;
    let url = format!("/api/posts/{}/reactions", post["id"]);

    let first = c.ada.post(&url, json!({"emoji": HEART})).await;
    first.assert_status(200);
    assert_eq!(
        first.json()["reactions"],
        json!([{"emoji": HEART, "count": 1, "mine": true, "users": ["ada"]}])
    );

    let second = c.grace.post(&url, json!({"emoji": HEART})).await.json();
    assert_eq!(second["reactions"][0]["count"], 2);
    assert_eq!(second["reactions"][0]["mine"], true);
    assert_eq!(second["reactions"][0]["users"], json!(["ada", "grace"]));

    let fire = c.grace.post(&url, json!({"emoji": FIRE})).await.json();
    let order: Vec<&str> = fire["reactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["emoji"].as_str().unwrap())
        .collect();
    assert_eq!(order, [HEART, FIRE]);

    let untoggled = c.grace.post(&url, json!({"emoji": HEART})).await.json();
    assert_eq!(
        untoggled["reactions"][0],
        json!({"emoji": HEART, "count": 1, "mine": false, "users": ["ada"]})
    );

    c.grace
        .post(&url, json!({"emoji": "\u{1f355}"}))
        .await
        .assert_detail(400, "Pick one of the club reactions");
    c.grace
        .post("/api/posts/999999/reactions", json!({"emoji": HEART}))
        .await
        .assert_detail(404, "No entry with that id");

    // Turning a reaction on records `reaction` {emoji, post_id, author_id}; turning it off
    // records nothing.
    let events = sqlx::query_as::<_, (i64, Option<i64>, i64, Option<i64>, Value)>(
        "SELECT club_id, actor_id, work_id, pick_id, payload FROM events
         WHERE kind = 'reaction' ORDER BY id",
    )
    .fetch_all(&c.app.db)
    .await
    .unwrap();
    assert_eq!(events.len(), 3);
    let expected = [
        (c.ada.id(), HEART),
        (c.grace.id(), HEART),
        (c.grace.id(), FIRE),
    ];
    for ((club, actor, work, pick, payload), (who, emoji)) in events.iter().zip(expected) {
        assert_eq!(*club, c.club.id);
        assert_eq!(*actor, Some(who));
        assert_eq!(*work, c.circe.id);
        assert_eq!(*pick, Some(c.pick));
        assert_eq!(
            *payload,
            json!({"emoji": emoji, "post_id": post["id"], "author_id": c.ada.id()})
        );
    }
}

/// parity: test_notes_and_meeting.py::test_deleting_a_note_removes_its_reactions
#[sqlx::test]
async fn deleting_a_post_removes_its_reactions(db: PgPool) {
    let c = club_with_pick(db).await;
    let post = c.post(&c.ada, json!({"body": "bye"})).await;
    c.ada
        .post(
            &format!("/api/posts/{}/reactions", post["id"]),
            json!({"emoji": THUMBS}),
        )
        .await
        .assert_status(200);
    c.ada
        .delete(&format!("/api/posts/{}", post["id"]))
        .await
        .assert_status(204);
    assert_eq!(
        c.app
            .scalar_i64("SELECT count(*) FROM post_reactions")
            .await,
        0
    );
    assert_eq!(c.app.scalar_i64("SELECT count(*) FROM posts").await, 0);
    // The events stay, with a dangling post id (parity).
    assert_eq!(
        c.app
            .scalar_i64("SELECT count(*) FROM events WHERE kind IN ('note_posted', 'reaction')")
            .await,
        2
    );
}

/// parity: test_diary.py::test_entry_on_the_current_pick_records_the_pick
///
/// Reasoned change (work-plan POSTS): `pick_id` survives only on club posts. A club post
/// records its pick; a diary entry on the pick's work is personal and has none.
#[sqlx::test]
async fn a_club_post_records_its_pick(db: PgPool) {
    let c = club_with_pick(db).await;
    let post = c.post(&c.ada, json!({"body": "Pick note"})).await;
    let row = sqlx::query_as::<_, (Option<i64>, Option<i64>, i64)>(
        "SELECT club_id, pick_id, work_id FROM posts WHERE id = $1",
    )
    .bind(post["id"].as_i64().unwrap())
    .fetch_one(&c.app.db)
    .await
    .unwrap();
    assert_eq!(row, (Some(c.club.id), Some(c.pick), c.circe.id));

    let entry = c
        .ada
        .post(
            &format!("/api/books/works/{}/posts", c.circe.id),
            json!({"body": "Side read"}),
        )
        .await
        .json();
    let row = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT club_id, pick_id FROM posts WHERE id = $1",
    )
    .bind(entry["id"].as_i64().unwrap())
    .fetch_one(&c.app.db)
    .await
    .unwrap();
    assert_eq!(row, (None, None));
    assert_eq!(entry["club"], Value::Null);
    // The pick thread holds only the club post; the work thread holds both.
    assert_eq!(
        bodies(&c.ada.get(&c.current()).await.json()["items"]),
        ["Pick note"]
    );
    let diary = c
        .ada
        .get(&format!("/api/books/works/{}/posts", c.circe.id))
        .await
        .json();
    assert_eq!(bodies(&diary["items"]), ["Pick note", "Side read"]);
    assert_eq!(diary["items"][0]["club"]["slug"], c.club.slug.as_str());
}

/// Every new club post, replies included, records `note_posted` {post_id}; diary entries record
/// nothing.
#[sqlx::test]
async fn club_posts_record_note_posted(db: PgPool) {
    let c = club_with_pick(db).await;
    let post = c.post(&c.ada, json!({"body": "Opening"})).await;
    let reply = c
        .post(&c.grace, json!({"body": "Agreed", "parent_id": post["id"]}))
        .await;
    let via_diary = c
        .grace
        .post(
            &format!("/api/books/works/{}/posts", c.circe.id),
            json!({"body": "Also agreed", "parent_id": post["id"]}),
        )
        .await;
    via_diary.assert_status(201);
    let via_diary = via_diary.json();
    assert_eq!(via_diary["club"]["slug"], c.club.slug.as_str());
    assert_eq!(via_diary["pick_id"], c.pick);
    c.grace
        .post(
            &format!("/api/books/works/{}/posts", c.circe.id),
            json!({"body": "My diary"}),
        )
        .await
        .assert_status(201);

    let events = sqlx::query_as::<_, (i64, Option<i64>, i64, Option<i64>, Value)>(
        "SELECT club_id, actor_id, work_id, pick_id, payload FROM events
         WHERE kind = 'note_posted' ORDER BY id",
    )
    .fetch_all(&c.app.db)
    .await
    .unwrap();
    let expected = [
        (c.ada.id(), &post),
        (c.grace.id(), &reply),
        (c.grace.id(), &via_diary),
    ];
    assert_eq!(events.len(), expected.len());
    for ((club, actor, work, pick, payload), (who, post)) in events.iter().zip(expected) {
        assert_eq!(
            (*club, *actor, *work, *pick),
            (c.club.id, Some(who), c.circe.id, Some(c.pick))
        );
        assert_eq!(*payload, json!({"post_id": post["id"]}));
    }
    assert_eq!(c.app.scalar_i64("SELECT count(*) FROM events").await, 3);

    let thread = c.ada.get(&c.current()).await.json();
    assert_eq!(bodies(&thread["items"]), ["Opening"]);
    assert_eq!(
        bodies(&thread["items"][0]["replies"]),
        ["Agreed", "Also agreed"]
    );
}

/// A post may name a milestone of its pick: 400 `That milestone is not on this pick` for a
/// missing one, another pick's or another club's.
#[sqlx::test]
async fn posts_name_milestones_of_their_pick(db: PgPool) {
    let c = club_with_pick(db).await;
    let part_one = milestone(&c.app, &c.club, c.pick, "Part one").await;
    let post = c
        .post(
            &c.ada,
            json!({"body": "Chapter 3!", "milestone_id": part_one}),
        )
        .await;
    assert_eq!(post["milestone_id"], part_one);
    assert_eq!(
        c.ada.get(&c.current()).await.json()["items"][0]["milestone_id"],
        part_one
    );

    let achilles = c.app.work("The Song of Achilles").await;
    let other = c.app.user("other").await;
    let other_club = c.app.club(&other, "Other").await;
    let other_pick = set_pick(&c.app, &other_club, &c.circe).await;
    let foreign = milestone(&c.app, &other_club, other_pick, "Elsewhere").await;
    for bad in [999_999, foreign] {
        c.ada
            .post(&c.current(), json!({"body": "x", "milestone_id": bad}))
            .await
            .assert_detail(400, "That milestone is not on this pick");
    }
    let old_milestone = part_one;
    let new_pick = set_pick(&c.app, &c.club, &achilles).await;
    c.ada
        .post(
            &c.current(),
            json!({"body": "x", "milestone_id": old_milestone}),
        )
        .await
        .assert_detail(400, "That milestone is not on this pick");
    let current = milestone(&c.app, &c.club, new_pick, "Part one again").await;
    c.post(&c.ada, json!({"body": "ok", "milestone_id": current}))
        .await;
    // Deleting a milestone keeps its posts (posts_milestone_fk sets it to NULL).
    c.app
        .exec(&format!("DELETE FROM milestones WHERE id = {part_one}"))
        .await;
    let archived = c.ada.get(&c.of_pick(c.pick)).await.json();
    assert_eq!(archived["items"][0]["milestone_id"], Value::Null);
}

/// A pick id of another club under this club's path is a missing pick; a non-numeric one 400.
#[sqlx::test]
async fn picks_are_looked_up_inside_the_club(db: PgPool) {
    let c = club_with_pick(db).await;
    let other = c.app.user("other").await;
    let other_club = c.app.club(&other, "Other").await;
    c.app.join(&other_club, &c.ada, "member").await;
    let foreign = set_pick(&c.app, &other_club, &c.circe).await;
    c.ada
        .get(&c.of_pick(foreign))
        .await
        .assert_detail(404, "No club pick with that id");
    c.ada
        .post(&c.of_pick(foreign), json!({"body": "x"}))
        .await
        .assert_detail(404, "No club pick with that id");
    c.ada
        .get(&c.of_pick(999_999))
        .await
        .assert_detail(404, "No club pick with that id");
    c.ada
        .get(&format!("/api/clubs/{}/picks/x/posts", c.club.slug))
        .await
        .assert_detail(
            400,
            "Input should be a valid integer, unable to parse string as an integer",
        );
    // A reply's parent must be a post of this pick, in this club.
    let elsewhere = c
        .ada
        .post(
            &format!("/api/clubs/{}/pick/posts", other_club.slug),
            json!({"body": "Other club"}),
        )
        .await
        .json();
    let diary = c
        .ada
        .post(
            &format!("/api/books/works/{}/posts", c.circe.id),
            json!({"body": "Diary"}),
        )
        .await
        .json();
    for parent in [&elsewhere, &diary] {
        c.ada
            .post(
                &c.current(),
                json!({"body": "Reply", "parent_id": parent["id"]}),
            )
            .await
            .assert_detail(400, "That entry is not on this book");
    }
}

/// Admins and the owner of the post's club delete others' club posts (a tombstone when it has
/// replies); a plain member does not, and nobody moderates diary entries.
#[sqlx::test]
async fn club_admins_moderate_their_clubs_posts(db: PgPool) {
    let c = club_with_pick(db).await;
    let tom = c.app.user("tom").await;
    c.app.join(&c.club, &tom, "admin").await;
    let lone = c.post(&c.grace, json!({"body": "Off topic"})).await;
    tom.delete(&format!("/api/posts/{}", lone["id"]))
        .await
        .assert_status(204);
    let root = c.post(&c.grace, json!({"body": "Rude"})).await;
    c.post(
        &c.ada,
        json!({"body": "Calm down", "parent_id": root["id"]}),
    )
    .await;
    c.ada
        .delete(&format!("/api/posts/{}", root["id"]))
        .await
        .assert_status(204);
    let thread = c.grace.get(&c.current()).await.json();
    assert_eq!(thread["items"][0]["deleted"], true);
    assert_eq!(thread["items"][0]["author"], "grace");
    assert_eq!(bodies(&thread["items"][0]["replies"]), ["Calm down"]);

    // A plain member cannot delete the owner's post.
    let owners = c.post(&c.ada, json!({"body": "Owner's note"})).await;
    c.grace
        .delete(&format!("/api/posts/{}", owners["id"]))
        .await
        .assert_detail(403, "You can only delete your own entries");
    // Admins have no say over diary entries, even on the pick's work.
    let diary = c
        .grace
        .post(
            &format!("/api/books/works/{}/posts", c.circe.id),
            json!({"body": "My diary"}),
        )
        .await
        .json();
    tom.delete(&format!("/api/posts/{}", diary["id"]))
        .await
        .assert_detail(403, "You can only delete your own entries");
}

/// Posts on an ended pick stay editable, deletable and reactable (parity: no closed check).
#[sqlx::test]
async fn posts_of_an_ended_pick_stay_editable(db: PgPool) {
    let c = club_with_pick(db).await;
    let post = c.post(&c.ada, json!({"body": "Before"})).await;
    let achilles = c.app.work("The Song of Achilles").await;
    set_pick(&c.app, &c.club, &achilles).await;
    let path = format!("/api/posts/{}", post["id"]);
    c.ada
        .patch(&path, json!({"body": "After"}))
        .await
        .assert_status(200);
    c.grace
        .post(&format!("{path}/reactions"), json!({"emoji": HEART}))
        .await
        .assert_status(200);
    c.ada.delete(&path).await.assert_status(204);
}
