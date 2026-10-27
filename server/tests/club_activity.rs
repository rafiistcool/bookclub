//! The club and home activity feeds (#45, conventions §6.4, endpoint-map §9,
//! docs/phase1/spec/club.md §1.24, §2.11, §5.7).
//!
//! Personal events (shelf, progress, quotes) and post events are inserted with SQL, as their
//! emitters (SHELF's and POSTS' routes) are other packages'. Pick, vote and join events come
//! from the routes.

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// Inserts an event and returns its id.
async fn event(
    app: &TestApp,
    club: Option<&TestClub>,
    actor: &TestClient,
    kind: &str,
    work: Option<&TestWork>,
    payload: Value,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO events (club_id, actor_id, kind, work_id, payload)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(club.map(|c| c.id))
    .bind(actor.id())
    .bind(kind)
    .bind(work.map(|w| w.id))
    .bind(payload)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// `who` joins `club` through a single-use invite of `admin`'s, which records `member_joined`.
async fn join_by_invite(club: &TestClub, admin: &TestClient, who: &TestClient) {
    let invite = admin
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "single_use"}),
        )
        .await;
    invite.assert_status(201);
    let code = invite.json()["code"].as_str().unwrap().to_owned();
    who.post(&format!("/api/invites/{code}/accept"), json!({}))
        .await
        .assert_status(200);
}

/// `(actor, kind)` of a feed page.
fn kinds(body: &Value) -> Vec<(Value, String)> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["actor"].clone(),
                item["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn ids(body: &Value) -> Vec<i64> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect()
}

/// parity: test_activity.py::test_feed_requires_auth
#[sqlx::test]
async fn feeds_require_a_session(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let anon = app.anon();
    anon.get("/api/activity")
        .await
        .assert_detail(401, "Not signed in");
    anon.get(&format!("/api/clubs/{}/activity", club.slug))
        .await
        .assert_detail(401, "Not signed in");
}

/// parity: test_activity.py::test_feed_records_the_club_story_newest_first
///
/// Joining a club (not registering) records `member_joined`. The unchanged-progress rule is
/// SHELF's emitter; the re-cast ballot is checked here.
#[sqlx::test]
async fn the_feed_tells_the_club_story_newest_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let grace = app.user("grace").await;
    let ada = app.user("ada").await;
    let club = app.club(&grace, "Readers").await;
    let path = |rest: &str| format!("/api/clubs/{}{rest}", club.slug);
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let achilles = app
        .ol_work("OL2W", "The Song of Achilles", "Madeline Miller")
        .await;
    join_by_invite(&club, &grace, &ada).await;
    let feed = ada.get(&path("/activity")).await.json();
    assert_eq!(kinds(&feed), [(json!("ada"), "member_joined".to_owned())]);
    app.exec(&format!(
        "UPDATE club_memberships SET role = 'admin' WHERE user_id = {}",
        ada.id()
    ))
    .await;

    event(
        &app,
        None,
        &ada,
        "shelf_added",
        Some(&circe),
        json!({"status": "want_to_read"}),
    )
    .await;
    event(
        &app,
        None,
        &ada,
        "shelf_moved",
        Some(&circe),
        json!({"status": "currently_reading"}),
    )
    .await;
    event(
        &app,
        None,
        &ada,
        "progress",
        Some(&circe),
        json!({"progress": 50}),
    )
    .await;
    app.exec(&format!(
        "INSERT INTO shelf_entries (user_id, work_id, status, rating, take)
         VALUES ({}, {}, 'finished', 4, 'Great')",
        ada.id(),
        circe.id
    ))
    .await;
    event(
        &app,
        None,
        &ada,
        "shelf_finished",
        Some(&circe),
        json!({"rating": 4, "take": "Great"}),
    )
    .await;
    let pick = ada
        .put(&path("/pick"), json!({"work_id": achilles.id}))
        .await
        .json();
    let pick_id = pick["id"].as_i64().unwrap();
    let post_id: i64 = sqlx::query_scalar(
        "INSERT INTO posts (work_id, author_id, club_id, pick_id, body)
         VALUES ($1, $2, $3, $4, 'Chapter one!') RETURNING id",
    )
    .bind(achilles.id)
    .bind(ada.id())
    .bind(club.id)
    .bind(pick_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    for (kind, payload) in [
        ("note_posted", json!({"post_id": post_id})),
        (
            "reaction",
            json!({"emoji": "❤️", "post_id": post_id, "author_id": ada.id()}),
        ),
    ] {
        sqlx::query(
            "INSERT INTO events (club_id, actor_id, kind, work_id, pick_id, payload)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(club.id)
        .bind(ada.id())
        .bind(kind)
        .bind(achilles.id)
        .bind(pick_id)
        .bind(payload)
        .execute(&app.db)
        .await
        .unwrap();
    }
    let vote = ada
        .post(&path("/vote/nominations"), json!({"work_id": circe.id}))
        .await
        .json();
    let nomination = vote["nominations"][0]["id"].clone();
    for _ in 0..2 {
        ada.post(&path("/vote/cast"), json!({"nomination_id": nomination}))
            .await
            .assert_status(200);
    }
    app.exec(&format!(
        "DELETE FROM shelf_entries WHERE user_id = {}",
        ada.id()
    ))
    .await;
    event(
        &app,
        None,
        &ada,
        "shelf_removed",
        Some(&circe),
        json!({"status": "finished"}),
    )
    .await;

    let expected = [
        "shelf_removed",
        "voted",
        "nominated",
        "reaction",
        "note_posted",
        "pick_set",
        "shelf_finished",
        "progress",
        "shelf_moved",
        "shelf_added",
        "member_joined",
    ];
    for feed_path in [path("/activity"), "/api/activity".to_owned()] {
        let feed = ada.get(&feed_path).await;
        feed.assert_status(200);
        let feed = feed.json();
        let got: Vec<String> = kinds(&feed).into_iter().map(|(_, kind)| kind).collect();
        assert_eq!(got, expected, "{feed_path}");
        assert_eq!(feed["has_more"], false);
        assert_eq!(feed["timezone"], "UTC");
        let items = feed["items"].as_array().unwrap();
        let find = |kind: &str| items.iter().find(|i| i["kind"] == kind).unwrap();
        let finished = find("shelf_finished");
        assert_eq!(finished["book"]["title"], "Circe");
        // The entry is gone now, but the actor always sees her own rating.
        assert_eq!(finished["payload"], json!({"rating": 4, "take": "Great"}));
        assert_eq!(finished["mine"], true);
        assert_eq!(finished["club"], Value::Null);
        assert_eq!(finished["pick_id"], Value::Null);
        let label = finished["created_label"].as_str().unwrap();
        assert!(label.ends_with(" UTC"), "{label}");
        assert!(finished["created_at"].as_str().unwrap().ends_with('Z'));
        assert_eq!(find("progress")["payload"], json!({"progress": 50}));
        let reaction = find("reaction");
        assert_eq!(
            reaction["payload"],
            json!({"emoji": "❤️", "post_id": post_id, "author": "ada"})
        );
        let pick_set = find("pick_set");
        assert_eq!(pick_set["pick_id"], pick_id);
        assert_eq!(pick_set["book"]["title"], "The Song of Achilles");
        assert_eq!(
            pick_set["club"],
            json!({"slug": club.slug, "name": "Readers"})
        );
        assert_eq!(find("nominated")["payload"], json!({}));
        assert_eq!(find("member_joined")["book"], Value::Null);
    }
    // grace sees the same story, none of it hers.
    let feed = grace.get(&path("/activity")).await.json();
    assert_eq!(feed["items"].as_array().unwrap().len(), expected.len());
    assert!(
        feed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["mine"] == false)
    );
}

/// parity: test_activity.py::test_feed_paginates_and_filters_by_member
#[sqlx::test]
async fn the_feed_pages_and_filters_by_member(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let ada = app.user("ada").await;
    let club = app.club(&owner, "Readers").await;
    let feed = format!("/api/clubs/{}/activity", club.slug);
    join_by_invite(&club, &owner, &ada).await;
    for n in 0..5 {
        let work = app.work(&format!("Book {n}")).await;
        event(
            &app,
            None,
            &ada,
            "shelf_added",
            Some(&work),
            json!({"status": "want_to_read"}),
        )
        .await;
    }
    for path in [feed.clone(), "/api/activity".to_owned()] {
        let first = ada.get(&format!("{path}?limit=3")).await.json();
        assert_eq!(first["items"].as_array().unwrap().len(), 3);
        assert_eq!(first["has_more"], true);
        let last = ids(&first)[2];
        let second = ada
            .get(&format!("{path}?limit=3&before={last}"))
            .await
            .json();
        assert_eq!(second["items"].as_array().unwrap().len(), 3);
        assert_eq!(second["has_more"], false);
        assert!(ids(&first).iter().all(|id| !ids(&second).contains(id)));
        assert!(
            ids(&first)
                .iter()
                .chain(&ids(&second))
                .is_sorted_by(|a, b| a > b)
        );
    }

    let grace = app.user("grace").await;
    join_by_invite(&club, &owner, &grace).await;
    for path in [feed.clone(), "/api/activity".to_owned()] {
        let only = grace.get(&format!("{path}?username=grace")).await.json();
        assert_eq!(kinds(&only), [(json!("grace"), "member_joined".to_owned())]);
        assert_eq!(only["items"][0]["mine"], true);
        let adas = grace.get(&format!("{path}?username=ADA")).await.json();
        assert_eq!(adas["items"].as_array().unwrap().len(), 6);
        assert!(kinds(&adas).iter().all(|(actor, _)| actor == "ada"));
        let nobody = grace.get(&format!("{path}?username=nobody")).await.json();
        assert_eq!(
            nobody,
            json!({"items": [], "has_more": false, "timezone": "UTC"})
        );
        // An empty name is no filter.
        let all = grace.get(&format!("{path}?username=")).await.json();
        assert_eq!(all["items"].as_array().unwrap().len(), 7);
    }
}

/// The query is validated with pydantic's wording.
#[sqlx::test]
async fn feed_queries_are_validated(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    for path in [
        format!("/api/clubs/{}/activity", club.slug),
        "/api/activity".to_owned(),
    ] {
        for (query, detail) in [
            ("before=0", "Input should be greater than or equal to 1"),
            ("limit=0", "Input should be greater than or equal to 1"),
            ("limit=101", "Input should be less than or equal to 100"),
            (
                "limit=1.5",
                "Input should be a valid integer, unable to parse string as an integer",
            ),
            (
                "before=x",
                "Input should be a valid integer, unable to parse string as an integer",
            ),
            (
                "before=0&limit=0",
                "Input should be greater than or equal to 1; Input should be greater than or equal to 1",
            ),
        ] {
            ada.get(&format!("{path}?{query}"))
                .await
                .assert_detail(400, detail);
        }
        ada.get(&format!("{path}?limit=100"))
            .await
            .assert_status(200);
    }
}

/// A user in two clubs sees each club's events in its own feed, and both (each once, with its
/// club) in the home feed; personal events appear once however many clubs share them (#45
/// acceptance).
#[sqlx::test]
async fn a_user_in_two_clubs_sees_separate_feeds(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let mine = app.club(&ada, "Readers").await;
    let theirs = app.club(&grace, "Other").await;
    app.join(&mine, &grace, "member").await;
    app.join(&theirs, &ada, "member").await;
    let circe = app.work("Circe").await;
    let a = event(
        &app,
        Some(&mine),
        &ada,
        "nominated",
        Some(&circe),
        json!({}),
    )
    .await;
    let b = event(
        &app,
        Some(&theirs),
        &grace,
        "nominated",
        Some(&circe),
        json!({}),
    )
    .await;
    let p = event(
        &app,
        None,
        &grace,
        "shelf_added",
        Some(&circe),
        json!({"status": "want_to_read"}),
    )
    .await;

    let own = ada
        .get(&format!("/api/clubs/{}/activity", mine.slug))
        .await
        .json();
    assert_eq!(ids(&own), [p, a]);
    let other = ada
        .get(&format!("/api/clubs/{}/activity", theirs.slug))
        .await
        .json();
    assert_eq!(ids(&other), [p, b]);
    assert_eq!(other["items"][1]["club"]["name"], "Other");
    let home = ada.get("/api/activity").await.json();
    assert_eq!(ids(&home), [p, b, a]);
    let clubs: Vec<Value> = home["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["club"]["name"].clone())
        .collect();
    assert_eq!(clubs, [Value::Null, json!("Other"), json!("Readers")]);

    // Someone in neither club sees none of it.
    let zed = app.user("zed").await;
    app.club(&zed, "Third").await;
    assert_eq!(zed.get("/api/activity").await.json()["items"], json!([]));
    zed.get(&format!("/api/clubs/{}/activity", mine.slug))
        .await
        .assert_detail(404, "No such club");
    // The home-feed username filter covers whoever the caller can see; the club feed only the
    // club's members.
    assert_eq!(
        zed.get("/api/activity?username=ada").await.json()["items"],
        json!([])
    );
    let filtered = ada.get("/api/activity?username=grace").await.json();
    assert_eq!(ids(&filtered), [p, b]);
    let outsider = app.user("outsider").await;
    app.join(&theirs, &outsider, "member").await;
    let not_member = ada
        .get(&format!(
            "/api/clubs/{}/activity?username=outsider",
            mine.slug
        ))
        .await
        .json();
    assert_eq!(not_member["items"], json!([]));
}

/// Labels: club items in the club's zone; personal items in the club's zone in a club feed and in
/// the caller's zone in the home feed, whose `timezone` is the caller's.
#[sqlx::test]
async fn labels_use_the_club_zone_or_the_callers(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.exec(&format!(
        "UPDATE users SET timezone = 'Asia/Kolkata' WHERE id = {}",
        ada.id()
    ))
    .await;
    let res = ada
        .post(
            "/api/clubs",
            json!({"name": "Berlin", "timezone": "Europe/Berlin"}),
        )
        .await;
    res.assert_status(201);
    let club = TestClub {
        id: app
            .scalar_i64("SELECT id FROM clubs WHERE name = 'Berlin'")
            .await,
        slug: res.json()["slug"].as_str().unwrap().to_owned(),
        name: "Berlin".to_owned(),
    };
    let work = app.work("Circe").await;
    let club_event = event(&app, Some(&club), &ada, "nominated", Some(&work), json!({})).await;
    let personal = event(&app, None, &ada, "shelf_added", Some(&work), json!({})).await;
    app.exec(&format!(
        "UPDATE events SET created_at = '2026-08-22T17:00:00Z' WHERE id IN ({club_event}, {personal})"
    ))
    .await;
    app.exec(&format!(
        "UPDATE club_memberships SET joined_at = '2026-01-01T00:00:00Z' WHERE club_id = {}",
        club.id
    ))
    .await;

    let feed = ada
        .get(&format!("/api/clubs/{}/activity", club.slug))
        .await
        .json();
    assert_eq!(feed["timezone"], "Europe/Berlin");
    let labels: Vec<&str> = feed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["created_label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Sat 22 Aug 2026, 19:00 CEST"; 2]);
    let home = ada.get("/api/activity").await.json();
    assert_eq!(home["timezone"], "Asia/Kolkata");
    let labels: Vec<(i64, &str)> = home["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["id"].as_i64().unwrap(),
                i["created_label"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        labels,
        [
            (personal, "Sat 22 Aug 2026, 22:30 IST"),
            (club_event, "Sat 22 Aug 2026, 19:00 CEST")
        ]
    );
}

/// Payloads are served as stored, apart from the read-time rules; a stored payload with odd
/// values never breaks the feed.
#[sqlx::test]
async fn odd_payloads_do_not_break_the_feed(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let work = app.work("Circe").await;
    for payload in [
        json!({"quote_id": "seven", "excerpt": "x"}),
        json!({"quote_id": 1.5, "excerpt": "x"}),
        json!({"quote_id": 1e20, "excerpt": "x"}),
        json!({"quote_id": -3, "excerpt": "x"}),
    ] {
        event(&app, None, &ada, "quote_added", Some(&work), payload).await;
    }
    event(
        &app,
        Some(&club),
        &ada,
        "reaction",
        Some(&work),
        json!({"emoji": "🔥", "author_id": "x"}),
    )
    .await;
    let feed = grace
        .get(&format!("/api/clubs/{}/activity", club.slug))
        .await;
    feed.assert_status(200);
    // grace sees no quote (none is named), only the reaction, with no author.
    assert_eq!(kinds(&feed.json()), [(json!("ada"), "reaction".to_owned())]);
    assert_eq!(
        feed.json()["items"][0]["payload"],
        json!({"emoji": "🔥", "author": null})
    );
    // ada sees her own quote events as stored.
    let own = ada.get("/api/activity").await.json();
    assert_eq!(own["items"].as_array().unwrap().len(), 5);
}
