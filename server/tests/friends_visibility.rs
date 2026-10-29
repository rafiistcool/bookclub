//! What a friendship shows (FRIENDS, #57; docs/phase3/work-plan.md §4.4, §4.5): a friend is the
//! owner's audience for personal data through the policy functions, and the home feed shows a
//! friend's personal events from the moment the friendship began. Club data stays the club's:
//! a friend's club events never reach a viewer outside that club. A pending request grants
//! nothing, and an ended friendship takes everything back. `GET /api/friends` shows the caller
//! their own lists only, with requesters' pictures only where the caller could see them anyway.

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// `requester` asks `addressee` to be friends; nobody answers.
async fn ask(requester: &TestClient, addressee: &TestClient) {
    requester
        .post(
            "/api/friends/requests",
            json!({ "username": addressee.username() }),
        )
        .await
        .assert_status(200);
}

/// `requester` asks and `addressee` accepts: they are friends from now on.
async fn befriend(requester: &TestClient, addressee: &TestClient) {
    ask(requester, addressee).await;
    addressee
        .post(
            &format!("/api/friends/requests/{}/accept", requester.username()),
            json!({}),
        )
        .await
        .assert_status(200);
}

/// `a` removes their friend `b`.
async fn unfriend(a: &TestClient, b: &TestClient) {
    a.delete(&format!("/api/friends/list/{}", b.username()))
        .await
        .assert_status(200);
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
    befriend(&ada, &bob).await;
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
    befriend(&bob, &ada).await;
    ask(&cleo, &bob).await;
    ask(&bob, &dan).await;
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
    unfriend(&ada, &bob).await;
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
    befriend(&ada, &bob).await;
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
    befriend(&bob, &ada).await;
    ask(&cleo, &bob).await;
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

/// Gives `who` the profile picture `picture` (26 characters).
async fn give_avatar(app: &TestApp, who: &TestClient, picture: &str) {
    app.exec(&format!(
        "INSERT INTO media (public_id, kind, source, content_type, variants, width, height,
                            byte_size, sha256, uploaded_by, storage_key)
         VALUES ('{picture}', 'avatar', 'upload', 'image/webp', ARRAY['a'], 1, 1, 1,
                 sha256('{picture}'), {id}, 'avatars/{picture}');
         UPDATE users SET avatar_media_id = (SELECT id FROM media WHERE public_id = '{picture}')
         WHERE id = {id}",
        id = who.id()
    ))
    .await;
}

async fn friends_of(who: &TestClient) -> Value {
    let res = who.get("/api/friends").await;
    res.assert_status(200);
    res.json()
}

/// §4.5: a request shows the other account's picture only when the caller can see that account
/// anyway (a club mate): a reader only sees a picture they could report (ADM2). A friend's
/// picture always shows.
#[sqlx::test]
async fn requesters_pictures_show_only_where_they_are_visible(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cleo = app.user("cleo").await;
    let dan = app.user("dan").await;
    let ada_picture = "adaadaadaadaadaadaadaadaad";
    let dan_picture = "dandandandandandandandanda";
    give_avatar(&app, &ada, ada_picture).await;
    give_avatar(&app, &dan, dan_picture).await;
    let club = app.club(&cleo, "Cleo's").await;
    app.join(&club, &ada, "member").await;

    ask(&ada, &bob).await;
    ask(&ada, &cleo).await;
    ask(&bob, &dan).await;
    let picture = |out: &Value, list: &str, name: &str| {
        out[list]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["user"]["username"] == name)
            .unwrap_or_else(|| panic!("{name} not in {list}: {out}"))["user"]["avatar_url"]
            .clone()
    };
    // Bob shares nothing with Ada: her request carries no picture. Cleo is her club mate.
    assert_eq!(
        picture(&friends_of(&bob).await, "incoming", "ada"),
        Value::Null
    );
    assert_eq!(
        picture(&friends_of(&cleo).await, "incoming", "ada"),
        format!("/media/{ada_picture}/a").as_str()
    );
    // The same for the requests one sent: Ada sees Cleo's (none) and Bob's; Bob not Dan's.
    assert_eq!(
        picture(&friends_of(&bob).await, "outgoing", "dan"),
        Value::Null
    );
    assert_eq!(
        picture(&friends_of(&dan).await, "incoming", "bob"),
        Value::Null
    );
    // Once friends, the picture shows.
    dan.post("/api/friends/requests/bob/accept", json!({}))
        .await
        .assert_status(200);
    assert_eq!(
        picture(&friends_of(&bob).await, "friends", "dan"),
        format!("/media/{dan_picture}/a").as_str()
    );
    let text = friends_of(&bob).await.to_string();
    assert!(!text.contains(ada_picture), "{text}");
}

/// §6 answer 24: friend lists are never served to anyone else. Each caller sees only their own
/// lists; `GET /api/friends` takes no `?username=`.
#[sqlx::test]
async fn a_friend_list_is_never_served_to_anyone_else(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cleo = app.user("cleo").await;
    let dan = app.user("dan").await;
    let eve = app.user("eve").await;
    befriend(&ada, &bob).await;
    befriend(&ada, &cleo).await;
    ask(&dan, &ada).await;
    ada.put("/api/friends/blocks/eve", json!({}))
        .await
        .assert_status(200);
    let ada_lists = friends_of(&ada).await.to_string();
    for name in ["bob", "cleo", "dan", "eve"] {
        assert!(ada_lists.contains(&format!("\"{name}\"")), "{ada_lists}");
    }
    for (viewer, others) in [
        (&bob, ["cleo", "dan", "eve"]),
        (&cleo, ["bob", "dan", "eve"]),
        (&eve, ["bob", "cleo", "dan"]),
    ] {
        for path in ["/api/friends", "/api/friends?username=ada"] {
            let res = viewer.get(path).await;
            res.assert_status(200);
            let text = res.text();
            for other in others {
                assert!(
                    !text.contains(&format!("\"{other}\"")),
                    "{} sees {other} at {path}: {text}",
                    viewer.username()
                );
            }
        }
    }
    // Eve is not told about the block.
    assert!(!friends_of(&eve).await.to_string().contains("\"ada\""));
}

/// §4.5: up to three books each friend is currently reading, in the friend's shelf order, with
/// the full count and the progress. Requests grant nothing; an "only me" take never shows.
#[sqlx::test]
async fn a_friend_shows_three_current_books_in_shelf_order(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cleo = app.user("cleo").await;
    befriend(&ada, &bob).await;
    ask(&cleo, &bob).await;
    for (i, title) in ["One", "Two", "Three", "Four"].into_iter().enumerate() {
        let work = app.work(title).await;
        shelve(
            &bob,
            &work,
            json!({"status": "currently_reading", "progress": 10 * (i + 1)}),
        )
        .await;
    }
    let done = app.work("Done").await;
    shelve(
        &bob,
        &done,
        json!({"status": "finished", "rating": 1, "take": "Secret take",
               "rating_visibility": "only_me"}),
    )
    .await;
    // Bob's own shelf order of his current books.
    let shelf = bob.get("/api/shelf").await.json();
    let order: Vec<(String, Value)> = shelf["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["status"] == "currently_reading")
        .map(|item| {
            (
                item["book"]["title"].as_str().unwrap().to_owned(),
                item["progress"].clone(),
            )
        })
        .collect();
    assert_eq!(order.len(), 4);

    let out = friends_of(&ada).await;
    let friend = &out["friends"][0];
    assert_eq!(friend["user"]["username"], "bob");
    assert_eq!(friend["currently_reading_count"], 4);
    let shown: Vec<(String, Value)> = friend["currently_reading"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["book"]["title"].as_str().unwrap().to_owned(),
                item["progress"].clone(),
            )
        })
        .collect();
    assert_eq!(shown, order[..3]);
    assert!(!out.to_string().contains("Secret take"));
    // Cleo's pending request shows Bob nothing of that.
    let theirs = friends_of(&cleo).await;
    assert_eq!(theirs["friends"], json!([]));
    assert_eq!(
        theirs["outgoing"][0]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["created_at", "user"]
    );
}
