//! Creating, listing, reading, updating and deleting clubs (endpoint-map §3, #40).

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

fn slug_pattern() -> regex::Regex {
    regex::Regex::new("^[a-z0-9](?:[a-z0-9-]{1,62}[a-z0-9])$").unwrap()
}

async fn create(client: &TestClient, body: Value) -> TestResponse {
    client.post("/api/clubs", body).await
}

#[sqlx::test]
async fn creating_a_club_makes_the_caller_its_owner(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.exec(&format!(
        "UPDATE users SET timezone = 'Europe/Berlin' WHERE id = {}",
        ada.id()
    ))
    .await;
    let res = create(&ada, json!({"name": "  Thursday \n Readers  "})).await;
    res.assert_status(201);
    let body = res.json();
    let slug = body["slug"].as_str().unwrap();
    assert!(
        slug.starts_with("thursday-readers-") && slug.len() == 21,
        "{slug}"
    );
    assert!(slug_pattern().is_match(slug));
    let created_at = OffsetDateTime::parse(body["created_at"].as_str().unwrap(), &Rfc3339).unwrap();
    assert!(body["created_at"].as_str().unwrap().ends_with('Z'));
    assert!(
        (OffsetDateTime::now_utc() - created_at)
            .whole_seconds()
            .abs()
            < 60
    );
    assert_eq!(
        body,
        json!({
            "slug": slug,
            "name": "Thursday Readers",
            "timezone": "Europe/Berlin",
            "role": "owner",
            "member_count": 1,
            "created_at": body["created_at"],
            "current_pick": null,
        })
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM clubs c JOIN club_memberships m ON m.club_id = c.id
             WHERE c.slug = '{slug}' AND c.created_by = {0} AND m.user_id = {0} AND m.role = 'owner'
               AND m.invited_by IS NULL",
            ada.id()
        ))
        .await,
        1
    );
    // Creating records no activity; joining through an invite does.
    assert_eq!(app.scalar_i64("SELECT count(*) FROM events").await, 0);
    // An explicit zone wins over the creator's.
    let res = create(
        &ada,
        json!({"name": "Night Owls", "timezone": " America/New_York "}),
    )
    .await;
    res.assert_status(201);
    assert_eq!(res.json()["timezone"], "America/New_York");
}

#[sqlx::test]
async fn creating_a_club_needs_a_verified_email(db: PgPool) {
    let app = TestApp::new(db).await;
    let grace = app.user_unverified("grace").await;
    create(&grace, json!({"name": "Readers"}))
        .await
        .assert_detail(403, "Verify your email address first");
    // The verification check comes before the body is looked at.
    create(&grace, json!({}))
        .await
        .assert_detail(403, "Verify your email address first");
    create(&app.anon(), json!({"name": "Readers"}))
        .await
        .assert_detail(401, "Not signed in");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM clubs").await, 0);
}

#[sqlx::test]
async fn club_names_and_zones_are_validated(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for (body, detail) in [
        (json!({"name": ""}), "Give the club a name"),
        (json!({"name": " \t\n "}), "Give the club a name"),
        (
            json!({"name": "x".repeat(81)}),
            "Keep the club name to 80 characters",
        ),
        (
            json!({"name": "Readers", "timezone": "Mars/Olympus"}),
            "That is not a valid time zone",
        ),
        (
            json!({"name": "Readers", "timezone": "europe/berlin"}),
            "That is not a valid time zone",
        ),
        (
            json!({"name": "", "timezone": "Nowhere"}),
            "Give the club a name; That is not a valid time zone",
        ),
        (json!({}), "Field required"),
        (json!({"name": 7}), "Input should be a valid string"),
    ] {
        create(&ada, body.clone()).await.assert_detail(400, detail);
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM clubs").await, 0);
    let long = "ä".repeat(80);
    let res = create(&ada, json!({"name": long})).await;
    res.assert_status(201);
    assert_eq!(res.json()["name"], long);
    assert!(res.json()["slug"].as_str().unwrap().starts_with("aeaeae"));
}

#[sqlx::test]
async fn slugs_are_random_per_club_and_never_change(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = create(&ada, json!({"name": "Bücherwürmer"})).await.json();
    let second = create(&ada, json!({"name": "Bücherwürmer"})).await.json();
    let (a, b) = (
        first["slug"].as_str().unwrap(),
        second["slug"].as_str().unwrap(),
    );
    assert!(a.starts_with("buecherwuermer-") && b.starts_with("buecherwuermer-"));
    assert_ne!(a, b);
    let odd = create(&ada, json!({"name": "!!!"})).await.json();
    assert!(odd["slug"].as_str().unwrap().starts_with("club-"));

    let res = ada
        .patch(&format!("/api/clubs/{a}"), json!({"name": "Night Owls"}))
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["slug"], a);
    assert_eq!(res.json()["name"], "Night Owls");
    ada.get(&format!("/api/clubs/{a}")).await.assert_status(200);
}

#[sqlx::test]
async fn a_user_creates_at_most_ten_clubs(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let mut slugs = Vec::new();
    for n in 0..10 {
        let res = create(&ada, json!({"name": format!("Club {n}")})).await;
        res.assert_status(201);
        slugs.push(res.json()["slug"].as_str().unwrap().to_owned());
    }
    create(&ada, json!({"name": "Eleven"}))
        .await
        .assert_detail(409, "You can create at most 10 clubs");
    // Validation comes first.
    create(&ada, json!({"name": ""}))
        .await
        .assert_detail(400, "Give the club a name");
    // A club handed to someone else still counts: it is the created clubs that count.
    let club = &slugs[0];
    let res = ada.get(&format!("/api/clubs/{club}")).await;
    res.assert_status(200);
    app.exec(&format!(
        "INSERT INTO club_memberships (club_id, user_id, role)
         SELECT id, {}, 'member' FROM clubs WHERE slug = '{club}'",
        bob.id()
    ))
    .await;
    ada.post(
        &format!("/api/clubs/{club}/transfer"),
        json!({"username": "bob"}),
    )
    .await
    .assert_status(200);
    create(&ada, json!({"name": "Eleven"}))
        .await
        .assert_detail(409, "You can create at most 10 clubs");
    // A deleted club frees its place.
    ada.delete(&format!("/api/clubs/{}", slugs[1]))
        .await
        .assert_status(204);
    create(&ada, json!({"name": "Eleven"}))
        .await
        .assert_status(201);
    // Another user has their own ten.
    create(&bob, json!({"name": "Bob's"}))
        .await
        .assert_status(201);
}

/// Concurrent creations by one user near the limit: the user row lock serialises the count, so
/// exactly the free places are used.
#[sqlx::test]
async fn concurrent_creations_cannot_exceed_the_limit(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for n in 0..8 {
        create(&ada, json!({"name": format!("Club {n}")}))
            .await
            .assert_status(201);
    }
    let attempts = (0..6).map(|n| {
        let ada = ada.clone();
        tokio::spawn(async move { create(&ada, json!({"name": format!("Race {n}")})).await })
    });
    let mut statuses = Vec::new();
    for attempt in attempts {
        statuses.push(attempt.await.unwrap().status.as_u16());
    }
    statuses.sort_unstable();
    assert_eq!(statuses, [201, 201, 409, 409, 409, 409]);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM clubs WHERE created_by = {}",
            ada.id()
        ))
        .await,
        10
    );
}

#[sqlx::test]
async fn the_club_list_is_newest_join_first_with_the_current_pick(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let older = app.club(&bob, "Older").await;
    app.join(&older, &ada, "admin").await;
    app.exec(&format!(
        "UPDATE club_memberships SET joined_at = now() - interval '1 day' WHERE club_id = {}",
        older.id
    ))
    .await;
    let newer = app.club(&ada, "Newer").await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let old_pick = app.work("Old pick").await;
    app.exec(&format!(
        "INSERT INTO club_picks (club_id, work_id, ended_at) VALUES ({0}, {1}, now());
         INSERT INTO club_picks (club_id, work_id, meeting_at)
         VALUES ({0}, {2}, '2026-11-05T18:30:00Z')",
        older.id, old_pick.id, circe.id
    ))
    .await;
    let pick_id = app
        .scalar_i64(&format!(
            "SELECT id FROM club_picks WHERE club_id = {} AND ended_at IS NULL",
            older.id
        ))
        .await;

    let res = ada.get("/api/clubs").await;
    res.assert_status(200);
    let items = res.json()["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 2);
    assert_eq!(items[0]["slug"], newer.slug);
    assert_eq!(items[0]["role"], "owner");
    assert_eq!(items[0]["current_pick"], Value::Null);
    assert_eq!(items[1]["slug"], older.slug);
    assert_eq!(items[1]["role"], "admin");
    assert_eq!(items[1]["member_count"], 2);
    let pick = &items[1]["current_pick"];
    assert_eq!(pick["id"], pick_id);
    assert_eq!(pick["meeting_at"], "2026-11-05T18:30:00Z");
    assert_eq!(pick["book"]["id"], circe.id);
    assert_eq!(pick["book"]["title"], "Circe");
    assert_eq!(pick["book"]["ol_work_key"], "/works/OL1W");
    assert_eq!(pick["book"]["authors"], "Madeline Miller");
    // GET /api/clubs/{slug} answers the same ClubOut.
    let one = ada.get(&format!("/api/clubs/{}", older.slug)).await;
    one.assert_status(200);
    assert_eq!(one.json(), items[1]);
    // Someone in no club gets an empty list.
    let res = app.user("cy").await.get("/api/clubs").await;
    assert_eq!(res.json(), json!({"items": []}));
}

#[sqlx::test]
async fn admins_rename_and_rezone_the_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let admin = app.user("admin").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &admin, "admin").await;
    let path = format!("/api/clubs/{}", club.slug);

    let res = admin
        .patch(
            &path,
            json!({"name": " Night  Owls ", "timezone": "Europe/Berlin"}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["name"], "Night Owls");
    assert_eq!(res.json()["timezone"], "Europe/Berlin");
    assert_eq!(res.json()["role"], "admin");
    // Absent and null leave a value as it is; {} is a no-op.
    for body in [json!({}), json!({"name": null, "timezone": null})] {
        let res = owner.patch(&path, body).await;
        res.assert_status(200);
        assert_eq!(res.json()["name"], "Night Owls");
        assert_eq!(res.json()["timezone"], "Europe/Berlin");
    }
    owner
        .patch(&path, json!({"name": "", "timezone": "UTC+2"}))
        .await
        .assert_detail(400, "Give the club a name; That is not a valid time zone");
    owner
        .patch(&path, json!({"name": "y".repeat(81)}))
        .await
        .assert_detail(400, "Keep the club name to 80 characters");
    let res = owner.patch(&path, json!({"timezone": "UTC"})).await;
    assert_eq!(res.json()["timezone"], "UTC");
    assert_eq!(res.json()["name"], "Night Owls");
}

#[sqlx::test]
async fn deleting_a_club_removes_everything_in_it(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let member = app.user("member").await;
    let club = app.club(&owner, "Readers").await;
    let other = app.club(&owner, "Other").await;
    app.join(&club, &member, "member").await;
    let work = app.work("Circe").await;
    owner
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "link"}),
        )
        .await
        .assert_status(201);
    app.exec(&format!(
        "WITH pick AS (
             INSERT INTO club_picks (club_id, work_id) VALUES ({c}, {w}) RETURNING id
         ), post AS (
             INSERT INTO posts (work_id, author_id, club_id, pick_id, body)
             SELECT {w}, {m}, {c}, id, 'Hello' FROM pick RETURNING id
         )
         INSERT INTO events (club_id, actor_id, kind, work_id, pick_id)
         SELECT {c}, {m}, 'pick_set', {w}, id FROM pick;
         INSERT INTO club_removals (club_id, user_id) VALUES ({c}, {o});",
        c = club.id,
        w = work.id,
        m = member.id(),
        o = app.user("removed").await.id()
    ))
    .await;

    owner
        .delete(&format!("/api/clubs/{}", club.slug))
        .await
        .assert_status(204);
    for table in [
        "clubs",
        "club_memberships",
        "club_invites",
        "club_removals",
        "club_picks",
        "posts",
        "events",
    ] {
        let column = if table == "clubs" { "id" } else { "club_id" };
        assert_eq!(
            app.scalar_i64(&format!(
                "SELECT count(*) FROM {table} WHERE {column} = {}",
                club.id
            ))
            .await,
            0,
            "{table}"
        );
    }
    member
        .get(&format!("/api/clubs/{}", club.slug))
        .await
        .assert_detail(404, "No such club");
    assert_eq!(member.get("/api/auth/me").await.json()["clubs"], json!([]));
    // The other club and the work are untouched.
    owner
        .get(&format!("/api/clubs/{}", other.slug))
        .await
        .assert_status(200);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {}",
            work.id
        ))
        .await,
        1
    );
}
