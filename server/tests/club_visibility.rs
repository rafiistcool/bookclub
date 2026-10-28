//! Personal data CLUB serves, across the viewer types of `tests/common/visibility.rs`
//! (conventions §6.5):
//! - the pick's readers: the club's members only, read through `bc_visible_shelf`, so an "only
//!   me" rating and take are hidden from everyone but the owner, while `dnf_reason` and
//!   `progress` stay visible (work-plan appendix "club" 11);
//! - the activity feeds (conventions §6.4): `shelf_finished` rating redaction, hidden
//!   `quote_added` events, and personal events from `joined_at` on, for current members only.

mod common;

use common::visibility::Viewers;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn shelve(app: &TestApp, who: &TestClient, work: &TestWork, columns: &str, values: &str) {
    app.exec(&format!(
        "INSERT INTO shelf_entries (user_id, work_id, {columns}) VALUES ({}, {}, {values})",
        who.id(),
        work.id
    ))
    .await;
}

fn readers(pick: &Value) -> Vec<(String, Value, Value, Value, Value)> {
    pick["readers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["username"].as_str().unwrap().to_owned(),
                r["status"].clone(),
                r["rating"].clone(),
                r["take"].clone(),
                json!([r["dnf_reason"], r["progress"]]),
            )
        })
        .collect()
}

fn name(n: &str) -> String {
    n.to_owned()
}

#[sqlx::test]
async fn pick_readers_are_club_members_with_only_me_ratings_redacted(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let dnf = app.user("dnf").await;
    app.join(&v.shared, &dnf, "member").await;
    shelve(
        &app,
        &v.owner,
        &circe,
        "status, rating, rating_visibility, take",
        "'finished', 5, 'only_me', 'Loved it'",
    )
    .await;
    shelve(
        &app,
        &v.co_member,
        &circe,
        "status, progress",
        "'currently_reading', 40",
    )
    .await;
    shelve(
        &app,
        &dnf,
        &circe,
        "status, dnf_reason, rating_visibility",
        "'did_not_finish', 'Too slow', 'only_me'",
    )
    .await;
    // Not members of the club (any more): never readers there.
    shelve(&app, &v.other_club, &circe, "status", "'want_to_read'").await;
    shelve(&app, &v.former, &circe, "status", "'finished'").await;

    let shared_pick = format!("/api/clubs/{}/pick", v.shared.slug);
    v.owner
        .put(&shared_pick, json!({"work_id": circe.id}))
        .await
        .assert_status(200);

    let seen = v.co_member.get(&shared_pick).await.json()["pick"].clone();
    assert_eq!(
        readers(&seen),
        [
            (
                name("comember"),
                json!("currently_reading"),
                Value::Null,
                json!(""),
                json!(["", 40])
            ),
            (
                name("dnf"),
                json!("did_not_finish"),
                Value::Null,
                json!(""),
                json!(["Too slow", null])
            ),
            (
                name("owner"),
                json!("finished"),
                Value::Null,
                json!(""),
                json!(["", null])
            ),
        ]
    );
    assert_eq!(seen["reading"], json!(["comember"]));
    assert_eq!(seen["finished"], json!(["owner"]));

    let own = v.owner.get(&shared_pick).await.json()["pick"].clone();
    assert_eq!(
        readers(&own)[0],
        (
            name("owner"),
            json!("finished"),
            json!(5),
            json!("Loved it"),
            json!(["", null])
        )
    );
    assert_eq!(own["on_shelf"], "finished");

    // An audience rating is visible to the club.
    app.exec(&format!(
        "UPDATE shelf_entries SET rating_visibility = 'audience' WHERE user_id = {}",
        v.owner.id()
    ))
    .await;
    let seen = v.co_member.get(&shared_pick).await.json()["pick"].clone();
    let owner_row = readers(&seen).into_iter().find(|r| r.0 == "owner").unwrap();
    assert_eq!((owner_row.2, owner_row.3), (json!(5), json!("Loved it")));

    // Strangers, members of another club and former members do not see the club at all.
    for viewer in v.strangers() {
        viewer
            .get(&shared_pick)
            .await
            .assert_detail(404, "No such club");
    }

    // In the other club, the owner (whom the co-member can see through `shared`) is no reader:
    // readers are that club's members only.
    let other_pick = format!("/api/clubs/{}/pick", v.other.slug);
    v.other_club
        .put(&other_pick, json!({"work_id": circe.id}))
        .await
        .assert_status(200);
    let there = v.co_member.get(&other_pick).await.json()["pick"].clone();
    let names: Vec<String> = readers(&there).into_iter().map(|r| r.0).collect();
    assert_eq!(names, ["comember", "otherclub"]);
}

/// The readers are sorted with the viewer first, then by username.
#[sqlx::test]
async fn readers_put_the_viewer_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let circe = app.work("Circe").await;
    let mut members = Vec::new();
    for username in ["zed", "grace", "bob"] {
        let user = app.user(username).await;
        app.join(&club, &user, "member").await;
        members.push(user);
    }
    for user in members.iter().chain([&ada]) {
        shelve(&app, user, &circe, "status", "'want_to_read'").await;
    }
    let path = format!("/api/clubs/{}/pick", club.slug);
    ada.put(&path, json!({"work_id": circe.id}))
        .await
        .assert_status(200);
    let grace = &members[1];
    let pick = grace.get(&path).await.json()["pick"].clone();
    let names: Vec<String> = readers(&pick).into_iter().map(|r| r.0).collect();
    assert_eq!(names, ["grace", "ada", "bob", "zed"]);
    assert_eq!(pick["reading"], json!([]));
    assert_eq!(pick["finished"], json!([]));
    assert_eq!(pick["readers"][0]["avatar_url"], Value::Null);
}

/// Inserts an event and returns its id.
async fn event(
    app: &TestApp,
    club: Option<&TestClub>,
    actor: &TestClient,
    kind: &str,
    work: &TestWork,
    payload: Value,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO events (club_id, actor_id, kind, work_id, payload)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(club.map(|c| c.id))
    .bind(actor.id())
    .bind(kind)
    .bind(work.id)
    .bind(payload)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// The item `id` of `viewer`'s feed at `path`, if it is there.
async fn feed_item(viewer: &TestClient, path: &str, id: i64) -> Option<Value> {
    let res = viewer.get(path).await;
    res.assert_status(200);
    res.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .cloned()
}

/// `shelf_finished` payloads lose `rating` and `take` for everyone but the actor while the
/// actor's entry is "only me" or gone; viewers who do not share a club with the actor never see
/// the event (conventions §6.4). This is the read-time backstop: the events and visibilities
/// here are written with SQL, past the write paths that keep an only-me rating out of every
/// stored payload (the tests below).
#[sqlx::test]
async fn finished_ratings_in_the_feeds_follow_the_entry(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    shelve(
        &app,
        &v.owner,
        &circe,
        "status, rating, rating_visibility, take",
        "'finished', 5, 'only_me', 'Loved it'",
    )
    .await;
    let id = event(
        &app,
        None,
        &v.owner,
        "shelf_finished",
        &circe,
        json!({"rating": 5, "take": "Loved it"}),
    )
    .await;
    let club_feed = format!("/api/clubs/{}/activity", v.shared.slug);
    let full = json!({"rating": 5, "take": "Loved it"});

    for path in [club_feed.as_str(), "/api/activity"] {
        let own = feed_item(&v.owner, path, id).await.unwrap();
        assert_eq!(own["payload"], full, "{path}");
        assert_eq!(own["mine"], true);
        for viewer in v.audience() {
            let seen = feed_item(viewer, path, id).await.unwrap();
            assert_eq!(seen["payload"], json!({}), "{path}");
            assert_eq!(seen["actor"], "owner");
        }
    }
    for viewer in v.strangers() {
        viewer
            .get(&club_feed)
            .await
            .assert_detail(404, "No such club");
        assert!(feed_item(viewer, "/api/activity", id).await.is_none());
    }

    // An audience rating is visible again, from the current entry.
    app.exec(&format!(
        "UPDATE shelf_entries SET rating_visibility = 'audience' WHERE user_id = {}",
        v.owner.id()
    ))
    .await;
    for path in [club_feed.as_str(), "/api/activity"] {
        let seen = feed_item(&v.co_member, path, id).await.unwrap();
        assert_eq!(seen["payload"], full, "{path}");
    }
    // A removed entry hides it again; the owner still sees hers.
    app.exec(&format!(
        "DELETE FROM shelf_entries WHERE user_id = {}",
        v.owner.id()
    ))
    .await;
    let seen = feed_item(&v.co_member, &club_feed, id).await.unwrap();
    assert_eq!(seen["payload"], json!({}));
    let own = feed_item(&v.owner, "/api/activity", id).await.unwrap();
    assert_eq!(own["payload"], full);
}

/// The stored payloads of `who`'s finished events on `work`, oldest first, with their ids.
async fn finished_events(app: &TestApp, who: &TestClient, work: &TestWork) -> Vec<(i64, Value)> {
    sqlx::query_as(
        "SELECT id, payload FROM events
         WHERE kind = 'shelf_finished' AND actor_id = $1 AND work_id = $2 ORDER BY id",
    )
    .bind(who.id())
    .bind(work.id)
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// What `viewer` sees of event `id` in both feeds of the shared club.
async fn seen_payloads(v: &Viewers, viewer: &TestClient, id: i64) -> Vec<Value> {
    let club_feed = format!("/api/clubs/{}/activity", v.shared.slug);
    let mut seen = Vec::new();
    for path in [club_feed.as_str(), "/api/activity"] {
        seen.push(feed_item(viewer, path, id).await.unwrap()["payload"].clone());
    }
    seen
}

/// Security review 2026-10-28 #2: an event outlives its entry. An only-me rating and take are
/// sealed to their reader when they are written, so adding the work again (as "start reading"
/// on the club page does, audience by default) shows them to nobody else. The reader still
/// sees their own.
#[sqlx::test]
async fn a_private_rating_stays_hidden_when_the_work_comes_back(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let added = v
        .owner
        .post(
            "/api/shelf",
            json!({"work_id": circe.id, "status": "finished", "rating": 1,
                   "take": "A gift I hated", "rating_visibility": "only_me"}),
        )
        .await;
    added.assert_status(201);
    let entry = added.json()["id"].clone();
    let events = finished_events(&app, &v.owner, &circe).await;
    assert_eq!(events.len(), 1);
    let (id, payload) = events[0].clone();
    assert_eq!(
        payload,
        json!({"rating": 1, "take": "A gift I hated", "private": true})
    );

    v.owner
        .delete(&format!("/api/shelf/{entry}"))
        .await
        .assert_status(204);
    v.owner
        .post(
            "/api/shelf",
            json!({"work_id": circe.id, "status": "currently_reading"}),
        )
        .await
        .assert_status(201);
    assert_eq!(
        seen_payloads(&v, &v.co_member, id).await,
        [json!({}), json!({})]
    );
    let own = json!({"rating": 1, "take": "A gift I hated"});
    assert_eq!(seen_payloads(&v, &v.owner, id).await, [own.clone(), own]);
}

/// A removed entry's events seal a rating that was public too: whatever the reader shows of the
/// work from now on comes from the new entry.
#[sqlx::test]
async fn a_removed_entry_seals_the_ratings_of_its_events(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let added = v
        .owner
        .post(
            "/api/shelf",
            json!({"work_id": circe.id, "status": "finished", "rating": 4, "take": "Fine"}),
        )
        .await;
    added.assert_status(201);
    let (id, payload) = finished_events(&app, &v.owner, &circe).await[0].clone();
    assert_eq!(payload, json!({"rating": 4, "take": "Fine"}));
    assert_eq!(
        seen_payloads(&v, &v.co_member, id).await,
        [payload.clone(), payload]
    );
    v.owner
        .delete(&format!("/api/shelf/{}", added.json()["id"]))
        .await
        .assert_status(204);
    assert_eq!(
        finished_events(&app, &v.owner, &circe).await[0].1,
        json!({"rating": 4, "take": "Fine", "private": true})
    );
    v.owner
        .post("/api/shelf", json!({"work_id": circe.id}))
        .await
        .assert_status(201);
    assert_eq!(
        seen_payloads(&v, &v.co_member, id).await,
        [json!({}), json!({})]
    );
}

/// A take written while the rating was only-me stays hidden when the reader later rewrites the
/// take and shares the entry: the feed never serves the old, private text.
#[sqlx::test]
async fn a_take_written_while_only_me_stays_hidden_after_the_entry_opens_up(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let added = v
        .owner
        .post(
            "/api/shelf",
            json!({"work_id": circe.id, "status": "finished", "rating": 2,
                   "take": "Only for me", "rating_visibility": "only_me"}),
        )
        .await;
    added.assert_status(201);
    v.owner
        .patch(
            &format!("/api/shelf/{}", added.json()["id"]),
            json!({"take": "Worth a read", "rating_visibility": "audience"}),
        )
        .await
        .assert_status(200);
    let (id, _) = finished_events(&app, &v.owner, &circe).await[0].clone();
    assert_eq!(
        seen_payloads(&v, &v.co_member, id).await,
        [json!({}), json!({})]
    );
}

/// Making a shared rating only-me seals it in the stored events too, so no later change (a merge
/// onto an audience entry, a visibility flip) can show it to others again.
#[sqlx::test]
async fn making_a_rating_only_me_seals_it_in_the_events(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let added = v
        .owner
        .post(
            "/api/shelf",
            json!({"work_id": circe.id, "status": "finished", "rating": 5, "take": "Loved it"}),
        )
        .await;
    added.assert_status(201);
    v.owner
        .patch(
            &format!("/api/shelf/{}", added.json()["id"]),
            json!({"rating_visibility": "only_me"}),
        )
        .await
        .assert_status(200);
    let (id, payload) = finished_events(&app, &v.owner, &circe).await[0].clone();
    assert_eq!(
        payload,
        json!({"rating": 5, "take": "Loved it", "private": true})
    );
    v.owner
        .patch(
            &format!("/api/shelf/{}", added.json()["id"]),
            json!({"rating_visibility": "audience"}),
        )
        .await
        .assert_status(200);
    assert_eq!(
        seen_payloads(&v, &v.co_member, id).await,
        [json!({}), json!({})]
    );
}

/// A work merge moves the loser's events to the winner. An event of an only-me entry (here as a
/// legacy event would carry its rating) must not show once it hangs off the user's audience
/// entry of the winner.
#[sqlx::test]
async fn a_merge_does_not_show_the_rating_of_an_only_me_entry(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let winner = app.work("Circe").await;
    let loser = app.work("Circe (duplicate)").await;
    shelve(
        &app,
        &v.owner,
        &winner,
        "status, rating, rating_visibility, take",
        "'finished', 4, 'audience', 'Good'",
    )
    .await;
    shelve(
        &app,
        &v.owner,
        &loser,
        "status, rating, rating_visibility, take, updated_at",
        "'finished', 1, 'only_me', 'Secret', now() - interval '1 day'",
    )
    .await;
    let id = event(
        &app,
        None,
        &v.owner,
        "shelf_finished",
        &loser,
        json!({"rating": 1, "take": "Secret"}),
    )
    .await;
    let mut conn = app.db.acquire().await.unwrap();
    bookclub_server::catalog::merge_works(&mut conn, loser.id, winner.id)
        .await
        .unwrap();
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM shelf_entries
             WHERE user_id = {} AND work_id = {} AND rating_visibility = 'audience'",
            v.owner.id(),
            winner.id
        ))
        .await,
        1
    );
    assert_eq!(
        seen_payloads(&v, &v.co_member, id).await,
        [json!({}), json!({})]
    );
}

/// A quote added only-me with one text, then rewritten and shared, shows the text it has now:
/// the feed reads the quote, not the excerpt stored when it was added.
#[sqlx::test]
async fn a_shared_quote_shows_its_current_text(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let created = v
        .owner
        .post(
            "/api/quotes",
            json!({"work_id": circe.id, "body": "A note only for me", "page": 12,
                   "visibility": "only_me"}),
        )
        .await;
    created.assert_status(201);
    let quote_id = created.json()["id"].as_i64().unwrap();
    let body =
        "Witches are not known for their humility, it is said; that is why we are good at it";
    v.owner
        .patch(
            &format!("/api/quotes/{quote_id}"),
            json!({"body": body, "page": 40, "visibility": "audience"}),
        )
        .await
        .assert_status(200);
    let id = app
        .scalar_i64(&format!(
            "SELECT id FROM events WHERE kind = 'quote_added' AND actor_id = {}",
            v.owner.id()
        ))
        .await;
    let excerpt: String = body.chars().take(80).collect();
    let shown = json!({"excerpt": excerpt, "page": 40, "quote_id": quote_id});
    assert_eq!(
        seen_payloads(&v, &v.co_member, id).await,
        [shown.clone(), shown.clone()]
    );
    assert_eq!(
        seen_payloads(&v, &v.owner, id).await,
        [shown.clone(), shown]
    );
}

/// `quote_added` is shown to others only while its quote exists and is not "only me"; a legacy
/// event without a `quote_id`, or one naming someone else's quote, is the actor's alone.
#[sqlx::test]
async fn quote_events_follow_the_quote(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let quote = |who: &TestClient, visibility: &str| {
        let sql = format!(
            "INSERT INTO quotes (user_id, work_id, body, visibility)
             VALUES ({}, {}, 'Witches are not', '{visibility}') RETURNING id",
            who.id(),
            circe.id
        );
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    let shared_quote = quote(&v.owner, "audience").await;
    let private_quote = quote(&v.owner, "only_me").await;
    let gone_quote = quote(&v.owner, "audience").await;
    let foreign_quote = quote(&v.co_member, "audience").await;
    let mut events = Vec::new();
    for payload in [
        json!({"quote_id": shared_quote, "excerpt": "Witches are not"}),
        json!({"quote_id": private_quote, "excerpt": "Witches are not"}),
        json!({"quote_id": gone_quote, "excerpt": "Witches are not"}),
        json!({"quote_id": foreign_quote, "excerpt": "Witches are not"}),
        json!({"excerpt": "Witches are not", "page": 3}),
    ] {
        events.push(event(&app, None, &v.owner, "quote_added", &circe, payload).await);
    }
    app.exec(&format!("DELETE FROM quotes WHERE id = {gone_quote}"))
        .await;
    let club_feed = format!("/api/clubs/{}/activity", v.shared.slug);

    for path in [club_feed.as_str(), "/api/activity"] {
        for id in &events {
            assert!(
                feed_item(&v.owner, path, *id).await.is_some(),
                "the owner misses {id} in {path}"
            );
        }
        let seen: Vec<bool> = {
            let mut seen = Vec::new();
            for id in &events {
                seen.push(feed_item(&v.co_member, path, *id).await.is_some());
            }
            seen
        };
        assert_eq!(seen, [true, false, false, false, false], "{path}");
    }
    for viewer in v.strangers() {
        for id in &events {
            assert!(feed_item(viewer, "/api/activity", *id).await.is_none());
        }
    }
    // The visible one keeps its payload.
    let shown = feed_item(&v.co_member, &club_feed, events[0])
        .await
        .unwrap();
    assert_eq!(
        shown["payload"],
        json!({"quote_id": shared_quote, "excerpt": "Witches are not"})
    );
}

/// A member's personal events reach a club feed from the moment they joined that club, and
/// only while they are a member; their club events stay. The home feed joins each shared club
/// on its own terms (work-plan appendix "club" 6).
#[sqlx::test]
async fn personal_events_show_from_joined_at(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let newcomer = app.user("newcomer").await;
    let at =
        |id: i64, when: &str| format!("UPDATE events SET created_at = '{when}' WHERE id = {id}");
    let before = event(&app, None, &newcomer, "shelf_added", &circe, json!({})).await;
    let after = event(&app, None, &newcomer, "shelf_added", &circe, json!({})).await;
    app.exec(&at(before, "2026-03-01T00:00:00Z")).await;
    app.exec(&at(after, "2026-05-01T00:00:00Z")).await;
    app.join(&v.shared, &newcomer, "member").await;
    app.exec(&format!(
        "UPDATE club_memberships SET joined_at = '2026-04-01T00:00:00Z' WHERE user_id = {}",
        newcomer.id()
    ))
    .await;
    let club_event = event(
        &app,
        Some(&v.shared),
        &newcomer,
        "nominated",
        &circe,
        json!({}),
    )
    .await;
    let club_feed = format!("/api/clubs/{}/activity", v.shared.slug);

    for path in [club_feed.as_str(), "/api/activity"] {
        assert!(
            feed_item(&v.co_member, path, before).await.is_none(),
            "{path}"
        );
        assert!(
            feed_item(&v.co_member, path, after).await.is_some(),
            "{path}"
        );
        assert!(
            feed_item(&v.co_member, path, club_event).await.is_some(),
            "{path}"
        );
    }
    // The newcomer always sees their own personal events in the home feed.
    assert!(
        feed_item(&newcomer, "/api/activity", before)
            .await
            .is_some()
    );
    assert!(feed_item(&newcomer, &club_feed, before).await.is_none());

    // The co-member is also in `other`: other_club's personal events reach the home feed
    // through it, never the `shared` feed.
    let theirs = event(&app, None, &v.other_club, "shelf_added", &circe, json!({})).await;
    assert!(
        feed_item(&v.co_member, "/api/activity", theirs)
            .await
            .is_some()
    );
    assert!(feed_item(&v.co_member, &club_feed, theirs).await.is_none());
    assert!(feed_item(&v.owner, "/api/activity", theirs).await.is_none());

    // After leaving, the personal events go; the club event stays.
    app.exec(&format!(
        "DELETE FROM club_memberships WHERE user_id = {}",
        newcomer.id()
    ))
    .await;
    for path in [club_feed.as_str(), "/api/activity"] {
        assert!(
            feed_item(&v.co_member, path, after).await.is_none(),
            "{path}"
        );
        assert!(
            feed_item(&v.co_member, path, club_event).await.is_some(),
            "{path}"
        );
    }
}
