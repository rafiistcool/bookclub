//! What a friendship shows (FRIENDS, #57; docs/phase3/work-plan.md §4.4): a friend is the
//! owner's audience for personal data through the policy functions, and the home feed shows a
//! friend's personal events from the moment the friendship began. Club data stays the club's:
//! a friend's club events never reach a viewer outside that club. A pending request grants
//! nothing, and an ended friendship takes everything back.
//!
//! The friendships are written with SQL here; the friend routes come with FRIENDS part 2, which
//! adds the `GET /api/friends` cases (requesters' avatars, other accounts' friends, the
//! currently-reading preview).

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// `requester` and `addressee` become friends now.
async fn befriend(app: &TestApp, requester: &TestClient, addressee: &TestClient) {
    app.exec(&format!(
        "INSERT INTO friendships (requester_id, addressee_id, accepted_at)
         VALUES ({}, {}, now())",
        requester.id(),
        addressee.id()
    ))
    .await;
}

/// `requester` asks `addressee`; nobody answers.
async fn ask(app: &TestApp, requester: &TestClient, addressee: &TestClient) {
    app.exec(&format!(
        "INSERT INTO friendships (requester_id, addressee_id) VALUES ({}, {})",
        requester.id(),
        addressee.id()
    ))
    .await;
}

/// Ends whatever row the pair has, as an unfriend or a block does.
async fn unfriend(app: &TestApp, a: &TestClient, b: &TestClient) {
    app.exec(&format!(
        "DELETE FROM friendships
         WHERE least(requester_id, addressee_id) = least({a}, {b})
           AND greatest(requester_id, addressee_id) = greatest({a}, {b})",
        a = a.id(),
        b = b.id()
    ))
    .await;
}

/// Shelves `work` through the route (which records the personal event).
async fn shelve(who: &TestClient, work: &TestWork, body: Value) {
    let mut full = json!({ "work_id": work.id });
    if let (Some(full), Value::Object(extra)) = (full.as_object_mut(), body) {
        full.extend(extra);
    }
    who.post("/api/shelf", full).await.assert_status(201);
}

/// The feed items of `actor` in `viewer`'s feed at `path`, as (kind, club name or null).
async fn events_of(viewer: &TestClient, path: &str, actor: &str) -> Vec<(String, Value)> {
    let res = viewer.get(path).await;
    res.assert_status(200);
    res.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["actor"] == actor)
        .map(|item| {
            (
                item["kind"].as_str().unwrap().to_owned(),
                item["club"]["name"].clone(),
            )
        })
        .collect()
}

/// §4.4: the friend branch of the home feed sits inside its personal-event group, so a friend's
/// club events reach only that club's members, with or without `?username=`; their personal
/// events reach the friend.
#[sqlx::test]
async fn a_friends_club_events_never_reach_a_viewer_outside_the_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cleo = app.user("cleo").await;
    let club = app.club(&bob, "Bob's Readers").await;
    app.join(&club, &cleo, "member").await;
    befriend(&app, &ada, &bob).await;
    let circe = app.work("Circe").await;
    let song = app.work("Song").await;
    bob.put(
        &format!("/api/clubs/{}/pick", club.slug),
        json!({"work_id": circe.id}),
    )
    .await
    .assert_status(200);
    shelve(&bob, &song, json!({"status": "currently_reading"})).await;

    // Cleo, in the club, sees both kinds; Ada, the friend, the personal one only.
    let all = events_of(&cleo, "/api/activity", "bob").await;
    assert!(
        all.contains(&("pick_set".to_owned(), json!("Bob's Readers"))),
        "{all:?}"
    );
    assert!(all.iter().any(|(_, club)| club.is_null()), "{all:?}");
    for path in ["/api/activity", "/api/activity?username=bob"] {
        let seen = events_of(&ada, path, "bob").await;
        assert!(!seen.is_empty(), "{path}");
        assert!(
            seen.iter().all(|(_, club)| club.is_null()),
            "{path}: a club event reached a friend outside the club: {seen:?}"
        );
    }
    ada.get(&format!("/api/clubs/{}/activity", club.slug))
        .await
        .assert_detail(404, "No such club");
}

/// Personal events of a friend show from the moment the friendship began, mirroring
/// `joined_at` for co-members; a pending request in either direction shows nothing, and an
/// ended friendship takes them back.
#[sqlx::test]
async fn friends_see_personal_events_from_the_friendship_on(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cleo = app.user("cleo").await;
    let dan = app.user("dan").await;
    let before = app.work("Before").await;
    let after = app.work("After").await;
    shelve(&bob, &before, json!({"status": "want_to_read"})).await;
    befriend(&app, &bob, &ada).await;
    ask(&app, &cleo, &bob).await;
    ask(&app, &bob, &dan).await;
    shelve(&bob, &after, json!({"status": "want_to_read"})).await;

    let titles = |feed: &Value| -> Vec<String> {
        feed["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["actor"] == "bob")
            .map(|item| item["book"]["title"].as_str().unwrap().to_owned())
            .collect()
    };
    for path in ["/api/activity", "/api/activity?username=bob"] {
        let res = ada.get(path).await;
        res.assert_status(200);
        assert_eq!(titles(&res.json()), ["After"], "{path}");
    }
    // Both sides of a pending request: nothing, and `?username=` answers like an unknown name.
    for viewer in [&cleo, &dan] {
        let res = viewer.get("/api/activity?username=bob").await;
        res.assert_status(200);
        assert_eq!(res.json()["items"], json!([]), "{}", viewer.username());
        assert!(titles(&viewer.get("/api/activity").await.json()).is_empty());
    }
    // An ended friendship takes them back.
    unfriend(&app, &ada, &bob).await;
    for path in ["/api/activity", "/api/activity?username=bob"] {
        let res = ada.get(path).await;
        res.assert_status(200);
        assert!(titles(&res.json()).is_empty(), "{path}");
    }
}

/// An "only me" rating and take stay the owner's for a friend too: on the shelf, the book page
/// (reader, mean and count) and in the home feed's `shelf_finished` payload.
#[sqlx::test]
async fn an_only_me_rating_never_reaches_a_friend(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    befriend(&app, &ada, &bob).await;
    let circe = app.work("Circe").await;
    shelve(
        &bob,
        &circe,
        json!({"status": "finished", "rating": 2, "take": "Not for me",
               "rating_visibility": "only_me"}),
    )
    .await;

    let shelf = ada.get("/api/shelf?username=bob").await;
    shelf.assert_status(200);
    let entry = shelf.json()["items"][0].clone();
    assert_eq!(
        (
            &entry["rating"],
            &entry["take"],
            &entry["rating_visibility"]
        ),
        (&Value::Null, &json!(""), &Value::Null)
    );
    let page = ada.get(&format!("/api/books/works/{}", circe.id)).await;
    page.assert_status(200);
    let page = page.json();
    assert_eq!(page["readers"][0]["username"], "bob");
    assert_eq!(page["readers"][0]["rating"], Value::Null);
    assert_eq!(
        (&page["rating_count"], &page["club_rating"]),
        (&json!(0), &Value::Null)
    );
    let feed = ada.get("/api/activity?username=bob").await;
    feed.assert_status(200);
    let finished: Vec<Value> = feed.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "shelf_finished")
        .map(|item| item["payload"].clone())
        .collect();
    assert_eq!(finished, [json!({})]);
    let text = format!("{}{page}{}", shelf.text(), feed.text());
    assert!(!text.contains("Not for me"));
}

/// A friend sees the owner's profile picture on their posts and may report it, as a co-member
/// may (`bc_can_see_user`, ADM2: a reader only sees a picture they can report); the sender of a
/// pending request does neither, and gets the 404 of an unknown picture.
#[sqlx::test]
async fn a_friend_sees_and_may_report_the_profile_picture(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cleo = app.user("cleo").await;
    befriend(&app, &bob, &ada).await;
    ask(&app, &cleo, &bob).await;
    let picture = "b0bb0bb0bb0bb0bb0bb0bb0bb0";
    app.exec(&format!(
        "INSERT INTO media (public_id, kind, source, content_type, variants, width, height,
                            byte_size, sha256, uploaded_by, storage_key)
         VALUES ('{picture}', 'avatar', 'upload', 'image/webp', ARRAY['a'], 1, 1, 1,
                 sha256('bob'), {bob}, 'avatars/b0/{picture}');
         UPDATE users SET avatar_media_id = (SELECT id FROM media WHERE public_id = '{picture}')
         WHERE id = {bob}",
        bob = bob.id()
    ))
    .await;
    let circe = app.work("Circe").await;
    bob.post(
        &format!("/api/books/works/{}/posts", circe.id),
        json!({"body": "An island"}),
    )
    .await
    .assert_status(201);
    let thread = ada
        .get(&format!("/api/books/works/{}/posts", circe.id))
        .await;
    thread.assert_status(200);
    assert_eq!(
        thread.json()["items"][0]["author_avatar_url"],
        format!("/media/{picture}/a").as_str()
    );
    let report = |who: &TestClient| {
        let who = who.clone();
        async move {
            who.post(
                &format!("/api/media/{picture}/reports"),
                json!({"reason": "offensive"}),
            )
            .await
        }
    };
    report(&ada).await.assert_status(201);
    report(&cleo).await.assert_detail(404, "Not found");
}
