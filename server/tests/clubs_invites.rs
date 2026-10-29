//! Club invites and sign-up invites: create, list, revoke, preview, accept, expiry and reuse,
//! removed members, the 20-member limit, concurrency and the `beta-invite` CLI (endpoint-map
//! §3, docs/phase1/conventions.md §5.3, §13).

mod common;

use std::net::IpAddr;
use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::time::timeout;

const INVALID: &str = "That invite code is not valid";

async fn invite(admin: &TestClient, club: &TestClub, body: Value) -> Value {
    let res = admin
        .post(&format!("/api/clubs/{}/invites", club.slug), body)
        .await;
    res.assert_status(201);
    res.json()
}

async fn link(admin: &TestClient, club: &TestClub) -> String {
    code_of(&invite(admin, club, json!({"kind": "link"})).await)
}

fn code_of(invite: &Value) -> String {
    invite["code"].as_str().unwrap().to_owned()
}

async fn accept(client: &TestClient, code: &str) -> TestResponse {
    client
        .post(&format!("/api/invites/{code}/accept"), json!({}))
        .await
}

fn days_from_now(timestamp: &Value) -> f64 {
    let at = OffsetDateTime::parse(timestamp.as_str().unwrap(), &Rfc3339).unwrap();
    (at - OffsetDateTime::now_utc()).as_seconds_f64() / 86_400.0
}

async fn use_count(app: &TestApp, code: &str) -> i64 {
    app.scalar_i64(&format!(
        "SELECT use_count::bigint FROM club_invites WHERE code = '{code}'"
    ))
    .await
}

async fn members(app: &TestApp, club: &TestClub) -> i64 {
    app.scalar_i64(&format!(
        "SELECT count(*) FROM club_memberships WHERE club_id = {}",
        club.id
    ))
    .await
}

/// Adds `n` members `filler1`… to the club directly.
async fn fill(app: &TestApp, club: &TestClub, n: usize) {
    app.exec(&format!(
        "WITH extra AS (
             INSERT INTO users (username, password_hash)
             SELECT 'filler' || i, 'x' FROM generate_series(1, {n}) AS i
             RETURNING id
         )
         INSERT INTO club_memberships (club_id, user_id, role)
         SELECT {}, id, 'member' FROM extra",
        club.id
    ))
    .await;
}

#[sqlx::test]
async fn admins_create_single_use_codes_and_links(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;

    let single = invite(&owner, &club, json!({"kind": "single_use"})).await;
    let code = code_of(&single);
    assert_eq!(code.len(), 10);
    assert!(
        code.chars()
            .all(|c| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(c)),
        "{code}"
    );
    assert_eq!(
        single,
        json!({
            "id": single["id"],
            "code": code,
            "url": format!("http://localhost:5173/join/{code}"),
            "created_by": "owner",
            "created_at": single["created_at"],
            "expires_at": single["expires_at"],
            "max_uses": 1,
            "use_count": 0,
            "used_by": [],
            "revoked": false,
            "active": true,
            "personal": false,
            "invitee": null,
        })
    );
    assert!((days_from_now(&single["expires_at"]) - 30.0).abs() < 0.01);
    assert!(single["created_at"].as_str().unwrap().ends_with('Z'));

    let link = invite(&owner, &club, json!({"kind": "link"})).await;
    assert_eq!(link["max_uses"], Value::Null);
    assert!((days_from_now(&link["expires_at"]) - 7.0).abs() < 0.01);
    let short = invite(&owner, &club, json!({"kind": "link", "expires_in_days": 1})).await;
    assert!((days_from_now(&short["expires_at"]) - 1.0).abs() < 0.01);
    let long = invite(
        &owner,
        &club,
        json!({"kind": "single_use", "expires_in_days": 2}),
    )
    .await;
    assert!((days_from_now(&long["expires_at"]) - 2.0).abs() < 0.01);
    assert_ne!(code_of(&link), code);

    let path = format!("/api/clubs/{}/invites", club.slug);
    for (body, detail) in [
        (json!({}), "Field required"),
        (json!({"kind": "forever"}), "Invalid value"),
        (
            json!({"kind": "link", "expires_in_days": 0}),
            "Input should be greater than or equal to 1",
        ),
        (
            json!({"kind": "link", "expires_in_days": 31}),
            "Input should be less than or equal to 30",
        ),
        (
            json!({"kind": "link", "expires_in_days": 1.5}),
            "Input should be a valid integer",
        ),
    ] {
        owner.post(&path, body).await.assert_detail(400, detail);
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM club_invites").await, 4);
}

#[sqlx::test]
async fn admins_list_every_invite_of_the_club_newest_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let admin = app.user("admin").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &admin, "admin").await;
    let other = app.club(&owner, "Other").await;
    let first = invite(&owner, &club, json!({"kind": "single_use"})).await;
    let second = invite(&admin, &club, json!({"kind": "link"})).await;
    invite(&owner, &other, json!({"kind": "link"})).await;
    let ada = app.user("ada").await;
    let cy = app.user("cy").await;
    accept(&ada, &code_of(&second)).await.assert_status(200);
    accept(&cy, &code_of(&second)).await.assert_status(200);
    // Same creation time: the id breaks the tie.
    app.exec("UPDATE club_invites SET created_at = '2026-10-01T00:00:00Z'")
        .await;

    let res = admin
        .get(&format!("/api/clubs/{}/invites", club.slug))
        .await;
    res.assert_status(200);
    let items = res.json()["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 2);
    assert_eq!(items[0]["id"], second["id"]);
    assert_eq!(items[0]["created_by"], "admin");
    assert_eq!(items[0]["use_count"], 2);
    assert_eq!(items[0]["used_by"], json!(["ada", "cy"]));
    assert_eq!(items[0]["active"], true);
    assert_eq!(items[1]["id"], first["id"]);
    assert_eq!(items[1]["created_by"], "owner");
    assert_eq!(items[1]["used_by"], json!([]));
    // Members who are not admins see none of them.
    ada.get(&format!("/api/clubs/{}/invites", club.slug))
        .await
        .assert_detail(403, "Only club admins can do that");
}

#[sqlx::test]
async fn revoked_invites_stop_working(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let other = app.club(&owner, "Other").await;
    let created = invite(&owner, &club, json!({"kind": "link"})).await;
    let foreign = invite(&owner, &other, json!({"kind": "link"})).await;
    let id = created["id"].as_i64().unwrap();
    let path = |id: &str| format!("/api/clubs/{}/invites/{id}", club.slug);

    owner
        .delete(&path(&id.to_string()))
        .await
        .assert_status(204);
    owner
        .delete(&path(&id.to_string()))
        .await
        .assert_status(204);
    for missing in [foreign["id"].to_string(), "999999".to_owned()] {
        owner
            .delete(&path(&missing))
            .await
            .assert_detail(404, "No such invite");
    }
    owner.delete(&path("abc")).await.assert_detail(
        400,
        "Input should be a valid integer, unable to parse string as an integer",
    );
    let listed = owner
        .get(&format!("/api/clubs/{}/invites", club.slug))
        .await
        .json();
    assert_eq!(listed["items"][0]["revoked"], true);
    assert_eq!(listed["items"][0]["active"], false);

    let code = code_of(&created);
    let ada = app.user("ada").await;
    accept(&ada, &code).await.assert_detail(400, INVALID);
    app.anon()
        .get(&format!("/api/invites/{code}"))
        .await
        .assert_detail(404, INVALID);
    assert_eq!(use_count(&app, &code).await, 0);
    // The other club's invite was not touched.
    accept(&ada, &code_of(&foreign)).await.assert_status(200);
}

#[sqlx::test]
async fn the_preview_shows_what_a_valid_code_leads_to(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let code = link(&owner, &club).await;
    app.exec(
        "INSERT INTO club_invites (club_id, code, max_uses) VALUES (NULL, 'BETA2026', 1);
         INSERT INTO club_invites (club_id, code, max_uses, use_count) VALUES (NULL, 'USEDUP', 1, 1);
         INSERT INTO club_invites (club_id, code, expires_at)
         VALUES (NULL, 'EXPIRED', now() - interval '1 second');
         INSERT INTO club_invites (club_id, code, max_uses, revoked_at)
         VALUES (NULL, 'REVOKED', 5, now())",
    )
    .await;
    let anon = app.anon();

    let spaced = format!("{}-{}", code[..5].to_lowercase(), &code[5..]);
    for typed in [code.clone(), spaced, format!("%20{code}%20")] {
        let res = anon.get(&format!("/api/invites/{typed}")).await;
        res.assert_status(200);
        assert_eq!(
            res.json(),
            json!({"club": {"slug": club.slug, "name": "Readers"}, "kind": "club"})
        );
    }
    let res = anon.get("/api/invites/beta-2026").await;
    assert_eq!(res.json(), json!({"club": null, "kind": "signup"}));
    for code in ["NOPE", "USEDUP", "EXPIRED", "REVOKED", "%20"] {
        anon.get(&format!("/api/invites/{code}"))
            .await
            .assert_detail(404, INVALID);
    }
    // Looking at a preview uses nothing.
    assert_eq!(use_count(&app, &code).await, 0);
}

#[sqlx::test]
async fn previews_are_limited_per_ip(db: PgPool) {
    let app = TestApp::new(db).await;
    let anon = app.anon().with_ip("203.0.113.9".parse::<IpAddr>().unwrap());
    for _ in 0..60 {
        anon.get("/api/invites/NOPE").await.assert_status(404);
    }
    let res = anon.get("/api/invites/NOPE").await;
    res.assert_detail(429, "Slow down a little. Try again later.");
    assert!(res.header("retry-after").is_some());
    app.anon()
        .with_ip("203.0.113.10".parse::<IpAddr>().unwrap())
        .get("/api/invites/NOPE")
        .await
        .assert_status(404);
}

#[sqlx::test]
async fn accepting_an_invite_joins_the_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let admin = app.user("admin").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &admin, "admin").await;
    let code = code_of(&invite(&admin, &club, json!({"kind": "single_use"})).await);
    let ada = app.user("ada").await;

    let res = accept(&ada, &code.to_lowercase()).await;
    res.assert_status(200);
    let out = res.json();
    assert_eq!(out["slug"], club.slug);
    assert_eq!(out["role"], "member");
    assert_eq!(out["member_count"], 3);
    assert_eq!(out["current_pick"], Value::Null);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM club_memberships
             WHERE club_id = {} AND user_id = {} AND role = 'member' AND invited_by = {}",
            club.id,
            ada.id(),
            admin.id()
        ))
        .await,
        1
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM events WHERE club_id = {} AND actor_id = {}
               AND kind = 'member_joined' AND work_id IS NULL AND pick_id IS NULL
               AND payload = '{{}}'",
            club.id,
            ada.id()
        ))
        .await,
        1
    );
    assert_eq!(use_count(&app, &code).await, 1);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM club_invite_uses WHERE user_id = {}",
            ada.id()
        ))
        .await,
        1
    );
    let clubs = ada.get("/api/auth/me").await.json()["clubs"].clone();
    assert_eq!(clubs[0]["slug"], club.slug);
    // The single-use code is used up now.
    accept(&app.user("bob").await, &code)
        .await
        .assert_detail(400, INVALID);
}

#[sqlx::test]
async fn refused_invites_are_not_used(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let member = app.user("member").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &member, "member").await;
    let code = link(&owner, &club).await;
    app.exec("INSERT INTO club_invites (club_id, code, max_uses) VALUES (NULL, 'BETA2026', 5)")
        .await;
    let ada = app.user("ada").await;

    // Already a member (also the owner): 409, and the invite is not used.
    for client in [&member, &owner] {
        accept(client, &code)
            .await
            .assert_detail(409, "You are already in this club");
    }
    // A sign-up invite joins no club.
    accept(&ada, "BETA2026").await.assert_detail(400, INVALID);
    accept(&ada, "NOPE").await.assert_detail(400, INVALID);
    accept(&ada, "%20-%20").await.assert_detail(400, INVALID);
    assert_eq!(use_count(&app, &code).await, 0);
    assert_eq!(use_count(&app, "BETA2026").await, 0);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_invite_uses")
            .await,
        0
    );

    // Expired: an invite whose time ran out.
    app.exec(&format!(
        "UPDATE club_invites SET expires_at = now() - interval '1 second' WHERE code = '{code}'"
    ))
    .await;
    accept(&ada, &code).await.assert_detail(400, INVALID);
    assert_eq!(use_count(&app, &code).await, 0);
    assert_eq!(members(&app, &club).await, 2);
    let listed = owner
        .get(&format!("/api/clubs/{}/invites", club.slug))
        .await
        .json();
    assert_eq!(listed["items"][0]["code"], code.as_str());
    assert_eq!(listed["items"][0]["active"], false, "expired");
    assert_eq!(listed["items"][0]["revoked"], false);
    accept(&app.anon(), &code)
        .await
        .assert_detail(401, "Not signed in");
}

/// A link invite (`max_uses` NULL) is redeemable by any number of people until it expires.
#[sqlx::test]
async fn a_link_is_redeemable_while_it_has_no_use_limit(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let code = link(&owner, &club).await;
    for name in ["ada", "bob", "cy", "dee", "eve"] {
        accept(&app.user(name).await, &code)
            .await
            .assert_status(200);
    }
    assert_eq!(use_count(&app, &code).await, 5);
    let listed = owner
        .get(&format!("/api/clubs/{}/invites", club.slug))
        .await
        .json();
    assert_eq!(listed["items"][0]["active"], true);
    assert_eq!(listed["items"][0]["max_uses"], Value::Null);
    assert_eq!(members(&app, &club).await, 6);
}

/// Leaving records nothing: the same link lets a former member back in, and the log shows both
/// uses.
#[sqlx::test]
async fn a_member_who_left_rejoins_with_the_same_link(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let code = link(&owner, &club).await;
    let ada = app.user("ada").await;
    accept(&ada, &code).await.assert_status(200);
    ada.post(&format!("/api/clubs/{}/leave", club.slug), json!({}))
        .await
        .assert_status(204);
    accept(&ada, &code).await.assert_status(200);
    assert_eq!(use_count(&app, &code).await, 2);
    let listed = owner
        .get(&format!("/api/clubs/{}/invites", club.slug))
        .await
        .json();
    assert_eq!(listed["items"][0]["used_by"], json!(["ada", "ada"]));
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM events WHERE kind = 'member_joined' AND actor_id = {}",
            ada.id()
        ))
        .await,
        2
    );
}

/// A removed member cannot walk back in with an invite created before the removal (it is not
/// used), but can with a newer one, which deletes the removal row. The link they joined with is
/// revoked; an invite they never saw stays good for others.
#[sqlx::test]
async fn a_removed_member_needs_an_invite_made_after_the_removal(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let old = link(&owner, &club).await;
    let unseen = link(&owner, &club).await;
    let older_single = code_of(&invite(&owner, &club, json!({"kind": "single_use"})).await);
    let ada = app.user("ada").await;
    accept(&ada, &old).await.assert_status(200);
    owner
        .delete(&format!("/api/clubs/{}/members/ada", club.slug))
        .await
        .assert_status(204);

    for code in [&old, &unseen, &older_single] {
        accept(&ada, code).await.assert_detail(400, INVALID);
    }
    assert_eq!(use_count(&app, &old).await, 1);
    assert_eq!(use_count(&app, &unseen).await, 0);
    assert_eq!(use_count(&app, &older_single).await, 0);
    // The link Ada used is revoked; the one she never saw still works for others.
    accept(&app.user("bob").await, &old)
        .await
        .assert_detail(400, INVALID);
    accept(&app.user("cleo").await, &unseen)
        .await
        .assert_status(200);

    let new = link(&owner, &club).await;
    accept(&ada, &new).await.assert_status(200);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_removals").await,
        0
    );
    // Back in good standing: after leaving, an invite from before the removal works again.
    ada.post(&format!("/api/clubs/{}/leave", club.slug), json!({}))
        .await
        .assert_status(204);
    accept(&ada, &unseen).await.assert_status(200);
}

/// A removal revokes the club's active invites the removed person knows, so a second account
/// cannot use them either (through sign-up or accept): the invites they created and the ones
/// they redeemed, and, for an admin (who could list them all), every one. Expired and used-up
/// invites keep their state; another club's invites are untouched.
#[sqlx::test]
async fn a_removal_revokes_the_invites_the_removed_person_knows(db: PgPool) {
    let app = TestApp::with_env(db, &[("SIGNUP_MODE", "invite")]).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let other_club = app.club(&owner, "Elsewhere").await;
    let elsewhere = link(&owner, &other_club).await;

    // Mallory, an admin, made a link; she knows every code from the invite list.
    let mallory = app.user("mallory").await;
    app.join(&club, &mallory, "admin").await;
    let hers = link(&mallory, &club).await;
    let owners = link(&owner, &club).await;
    let expired = link(&owner, &club).await;
    let used_up = code_of(&invite(&owner, &club, json!({"kind": "single_use"})).await);
    accept(&app.user("dan").await, &used_up)
        .await
        .assert_status(200);
    app.exec(&format!(
        "UPDATE club_invites SET expires_at = now() - interval '1 minute' WHERE code = '{expired}'"
    ))
    .await;
    owner
        .delete(&format!("/api/clubs/{}/members/mallory", club.slug))
        .await
        .assert_status(204);

    let revoked = |code: String| {
        let app = &app;
        async move {
            app.scalar_i64(&format!(
                "SELECT count(*) FROM club_invites WHERE code = '{code}' AND revoked_at IS NOT NULL"
            ))
            .await
        }
    };
    assert_eq!(revoked(hers.clone()).await, 1);
    assert_eq!(revoked(owners.clone()).await, 1);
    assert_eq!(revoked(expired).await, 0, "already dead: left as it was");
    assert_eq!(revoked(used_up).await, 0, "already dead: left as it was");
    assert_eq!(revoked(elsewhere.clone()).await, 0, "another club");
    // A second account gets in with none of them.
    let res = app
        .anon()
        .post(
            "/api/auth/register",
            json!({"username": "mallory2", "email": "m2@example.test",
                   "password": PASSWORD, "invite_code": hers}),
        )
        .await;
    res.assert_detail(400, INVALID);
    accept(&app.user("mallory3").await, &owners)
        .await
        .assert_detail(400, INVALID);
    accept(&app.user("erin").await, &elsewhere)
        .await
        .assert_status(200);

    // A plain member: the invites they made or used, and, after a demotion, every invite they
    // could list as an admin (security review 2026-10-28 #9: demote-then-remove used to leave
    // those working for a second account).
    let bob = app.user("bob").await;
    app.join(&club, &bob, "admin").await;
    let bobs = link(&bob, &club).await;
    let seen = link(&owner, &club).await;
    owner
        .patch(
            &format!("/api/clubs/{}/members/bob", club.slug),
            json!({"role": "member"}),
        )
        .await
        .assert_status(200);
    let fresh = link(&owner, &club).await;
    let cleo = app.user("cleo").await;
    accept(&cleo, &fresh).await.assert_status(200);
    let untouched = link(&owner, &club).await;
    assert_eq!(revoked(bobs.clone()).await, 0);
    assert_eq!(revoked(seen.clone()).await, 0);
    owner
        .delete(&format!("/api/clubs/{}/members/bob", club.slug))
        .await
        .assert_status(204);
    assert_eq!(revoked(bobs.clone()).await, 1, "his own link");
    assert_eq!(
        revoked(seen.clone()).await,
        1,
        "listed while he was an admin"
    );
    assert_eq!(
        revoked(fresh.clone()).await,
        0,
        "created after his demotion"
    );
    owner
        .delete(&format!("/api/clubs/{}/members/cleo", club.slug))
        .await
        .assert_status(204);
    assert_eq!(revoked(fresh.clone()).await, 1, "the link she used");
    assert_eq!(revoked(untouched.clone()).await, 0);
    let res = app
        .anon()
        .post(
            "/api/auth/register",
            json!({"username": "bob2", "email": "b2@example.test",
                   "password": PASSWORD, "invite_code": seen}),
        )
        .await;
    res.assert_detail(400, INVALID);
    accept(&app.user("frank").await, &untouched)
        .await
        .assert_status(200);
}

/// Accepting is limited to 20 attempts per account and hour, valid code or not, so an account
/// cannot guess codes past the preview's per-IP limit (§10.1).
#[sqlx::test]
async fn accepts_are_limited_per_account(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let code = link(&owner, &club).await;
    let guesser = app.user("guesser").await;
    for n in 0..20 {
        accept(&guesser, &format!("GUESS{n:05}"))
            .await
            .assert_detail(400, INVALID);
    }
    let res = accept(&guesser, &code).await;
    res.assert_detail(429, "Slow down a little. Try again later.");
    assert!(res.header("retry-after").is_some());
    assert_eq!(use_count(&app, &code).await, 0);
    // Another account is not affected.
    accept(&app.user("ada").await, &code)
        .await
        .assert_status(200);
}

/// Of concurrent redemptions of a single-use invite exactly one succeeds.
#[sqlx::test]
async fn one_of_many_concurrent_redemptions_of_a_single_use_invite_wins(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let code = code_of(&invite(&owner, &club, json!({"kind": "single_use"})).await);
    let mut clients = Vec::new();
    for n in 0..8 {
        clients.push(app.user(&format!("racer{n}")).await);
    }
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(clients.len()));
    let tasks: Vec<_> = clients
        .into_iter()
        .map(|client| {
            let code = code.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                accept(&client, &code).await
            })
        })
        .collect();
    let mut wins = 0;
    for task in tasks {
        let res = task.await.unwrap();
        match res.status.as_u16() {
            200 => wins += 1,
            _ => {
                res.assert_detail(400, INVALID);
            }
        }
    }
    assert_eq!(wins, 1);
    assert_eq!(use_count(&app, &code).await, 1);
    assert_eq!(members(&app, &club).await, 2);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_invite_uses")
            .await,
        1
    );
}

/// A club holds 20 members: the 21st is refused with 409 and the invite use is rolled back, also
/// when two people race for the last place.
#[sqlx::test]
async fn a_club_holds_twenty_members(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    fill(&app, &club, 17).await;
    let code = link(&owner, &club).await;
    let ada = app.user("ada").await;
    accept(&ada, &code).await.assert_status(200);
    assert_eq!(members(&app, &club).await, 19);

    // Two people race for the last place with two different links, so only the club lock
    // orders them: both wait behind it, and the second counts the first.
    let other_code = link(&owner, &club).await;
    let mut hold = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM clubs WHERE id = $1 FOR UPDATE")
        .bind(club.id)
        .execute(&mut *hold)
        .await
        .unwrap();
    let mut tasks = Vec::new();
    for (n, (name, code)) in [("bob", &code), ("cy", &other_code)]
        .into_iter()
        .enumerate()
    {
        let client = app.user(name).await;
        let code = code.clone();
        tasks.push(tokio::spawn(async move { accept(&client, &code).await }));
        wait_for_waiters(&app, n as i64 + 1).await;
    }
    hold.commit().await.unwrap();
    let first = tasks.remove(0).await.unwrap();
    let second = tasks.remove(0).await.unwrap();
    first.assert_status(200);
    second.assert_detail(409, "This club is full");
    assert_eq!(use_count(&app, &other_code).await, 0);
    assert_eq!(members(&app, &club).await, 20);
    accept(&app.user("dee").await, &code)
        .await
        .assert_detail(409, "This club is full");
    assert_eq!(use_count(&app, &code).await, 2);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_invite_uses")
            .await,
        2
    );
    // A sign-up through the full club's invite is refused the same way.
    app.anon()
        .post(
            "/api/auth/register",
            json!({"username": "eve", "email": "eve@example.test", "password": PASSWORD,
                   "invite_code": code}),
        )
        .await
        .assert_detail(409, "This club is full");
    // A place frees up when someone leaves.
    ada.post(&format!("/api/clubs/{}/leave", club.slug), json!({}))
        .await
        .assert_status(204);
    accept(&app.user("fay").await, &code)
        .await
        .assert_status(200);
}

/// Lock order (clubs::invites): a join takes the inviter's row before the invite row. An
/// account deletion holds the inviter's row and then clears the invite's `created_by`; if the
/// join held the invite row first, the two would deadlock.
#[sqlx::test]
async fn a_join_waits_for_the_deletion_of_its_inviter_without_deadlock(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let inviter = app.user("inviter").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &inviter, "admin").await;
    let code = link(&inviter, &club).await;
    let ada = app.user("ada").await;

    // What `auth::deletion::delete_user` does to the inviter, held open.
    let mut deletion = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(inviter.id())
        .execute(&mut *deletion)
        .await
        .unwrap();
    let joining = {
        let ada = ada.clone();
        let code = code.clone();
        tokio::spawn(async move { accept(&ada, &code).await })
    };
    wait_for_a_lock(&app).await;
    // The join waits for the inviter's row and has not touched the invite yet.
    timeout(
        Duration::from_secs(5),
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(inviter.id())
            .execute(&mut *deletion),
    )
    .await
    .expect("the deletion did not wait for the join")
    .unwrap();
    deletion.commit().await.unwrap();

    let res = joining.await.unwrap();
    res.assert_status(200);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM club_memberships
             WHERE club_id = {} AND user_id = {} AND invited_by IS NULL",
            club.id,
            ada.id()
        ))
        .await,
        1
    );
    assert_eq!(use_count(&app, &code).await, 1);
}

/// The same for an inviter who created the club, handed it over and left: the deletion also
/// clears `clubs.created_by`, so it locks the club before the account row (§5.6). Otherwise the
/// join would hold the club and wait for the account row, and the deletion the other way round.
#[sqlx::test]
async fn a_join_through_the_link_of_a_former_creator_takes_turns_with_their_deletion(db: PgPool) {
    let app = TestApp::new(db).await;
    for (tag, join_first) in [("a", true), ("b", false)] {
        let creator = app.user(&format!("creator_{tag}")).await;
        let heir = app.user(&format!("heir_{tag}")).await;
        let club = app.club(&creator, "Readers").await;
        app.join(&club, &heir, "member").await;
        let code = link(&creator, &club).await;
        creator
            .post(
                &format!("/api/clubs/{}/transfer", club.slug),
                json!({"username": heir.username()}),
            )
            .await
            .assert_status(200);
        creator
            .post(&format!("/api/clubs/{}/leave", club.slug), json!({}))
            .await
            .assert_status(204);
        let ada = app.user(&format!("ada_{tag}")).await;
        let joining = {
            let ada = ada.clone();
            let code = code.clone();
            async move { accept(&ada, &code).await }
        };
        let deleting = {
            let creator = creator.clone();
            async move {
                creator
                    .delete_json("/api/auth/me", json!({"password": PASSWORD}))
                    .await
            }
        };
        let (joined, deleted) = if join_first {
            in_order(&app, &club, joining, deleting).await
        } else {
            let (deleted, joined) = in_order(&app, &club, deleting, joining).await;
            (joined, deleted)
        };
        joined.assert_status(200);
        deleted.assert_status(204);
        assert_eq!(
            app.scalar_i64(&format!(
                "SELECT count(*) FROM club_memberships
                 WHERE club_id = {} AND user_id = {} AND invited_by IS NULL",
                club.id,
                ada.id()
            ))
            .await,
            1
        );
        assert_eq!(use_count(&app, &code).await, 1);
    }
}

/// Sends `first`, waits until it queues behind a held lock on the club, then `second`, and
/// releases the lock: the two run in that order.
async fn in_order(
    app: &TestApp,
    club: &TestClub,
    first: impl Future<Output = TestResponse> + Send + 'static,
    second: impl Future<Output = TestResponse> + Send + 'static,
) -> (TestResponse, TestResponse) {
    let mut hold = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM clubs WHERE id = $1 FOR UPDATE")
        .bind(club.id)
        .execute(&mut *hold)
        .await
        .unwrap();
    let first = tokio::spawn(first);
    wait_for_waiters(app, 1).await;
    let second = tokio::spawn(second);
    wait_for_waiters(app, 2).await;
    hold.commit().await.unwrap();
    (first.await.unwrap(), second.await.unwrap())
}

async fn wait_for_a_lock(app: &TestApp) {
    wait_for_waiters(app, 1).await;
}

/// Waits until `n` sessions of the test database wait for a lock.
async fn wait_for_waiters(app: &TestApp, n: i64) {
    for _ in 0..500 {
        let waiting = app
            .scalar_i64(
                "SELECT count(*) FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event_type = 'Lock'",
            )
            .await;
        if waiting >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{n} requests never waited for a lock");
}

#[sqlx::test]
async fn instance_admins_mint_sign_up_invites(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    link(&owner, &club).await;

    let res = root.post("/api/admin/invites", json!({})).await;
    res.assert_status(201);
    let default = res.json();
    assert_eq!(default["max_uses"], 1);
    assert!((days_from_now(&default["expires_at"]) - 30.0).abs() < 0.01);
    assert_eq!(default["created_by"], "root");
    let code = code_of(&default);
    assert_eq!(default["url"], format!("http://localhost:5173/join/{code}"));

    let uses = root
        .post("/api/admin/invites", json!({"max_uses": 5}))
        .await
        .json();
    assert_eq!(uses["max_uses"], 5);
    assert_eq!(uses["expires_at"], Value::Null);
    let days = root
        .post("/api/admin/invites", json!({"expires_in_days": 90}))
        .await
        .json();
    assert_eq!(days["max_uses"], Value::Null);
    assert!((days_from_now(&days["expires_at"]) - 90.0).abs() < 0.01);
    for (body, detail) in [
        (
            json!({"max_uses": 0}),
            "Input should be greater than or equal to 1",
        ),
        (
            json!({"max_uses": 101}),
            "Input should be less than or equal to 100",
        ),
        (
            json!({"max_uses": 0, "expires_in_days": 91}),
            "Input should be greater than or equal to 1; Input should be less than or equal to 90",
        ),
    ] {
        root.post("/api/admin/invites", body)
            .await
            .assert_detail(400, detail);
    }

    let listed = root.get("/api/admin/invites").await;
    listed.assert_status(200);
    let ids: Vec<i64> = listed.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect();
    let expected: Vec<i64> = [&days, &uses, &default]
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, expected, "only sign-up invites, newest first");

    let res = app.anon().get(&format!("/api/invites/{code}")).await;
    assert_eq!(res.json(), json!({"club": null, "kind": "signup"}));
    let res = app
        .anon()
        .post(
            "/api/auth/register",
            json!({"username": "ada", "email": "ada@example.test", "password": PASSWORD,
                   "invite_code": code}),
        )
        .await;
    res.assert_status(201);
    assert_eq!(res.json()["clubs"], json!([]));
    let listed = root.get("/api/admin/invites").await.json();
    assert_eq!(listed["items"][2]["used_by"], json!(["ada"]));
    assert_eq!(listed["items"][2]["active"], false);
}

/// An instance admin revokes a sign-up invite (`DELETE /api/admin/invites/{invite_id}`): it no
/// longer unlocks sign-up, and revoking it again is harmless. A club's invite is not theirs to
/// revoke here (404 `No such invite`, unchanged), and an unknown id is 404 too.
#[sqlx::test]
async fn instance_admins_revoke_sign_up_invites(db: PgPool) {
    let app = TestApp::with_env(db, &[("SIGNUP_MODE", "invite")]).await;
    let root = app.admin("root").await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let clubs = invite(&owner, &club, json!({"kind": "link"})).await;
    let forever = root
        .post("/api/admin/invites", json!({"max_uses": 100}))
        .await
        .json();
    let id = forever["id"].as_i64().unwrap();
    root.delete(&format!("/api/admin/invites/{id}"))
        .await
        .assert_status(204);
    root.delete(&format!("/api/admin/invites/{id}"))
        .await
        .assert_status(204);
    let listed = root.get("/api/admin/invites").await.json();
    assert_eq!(listed["items"][0]["active"], false);
    assert_eq!(listed["items"][0]["revoked"], true);
    let res = app
        .anon()
        .post(
            "/api/auth/register",
            json!({"username": "ada", "email": "ada@example.test", "password": PASSWORD,
                   "invite_code": code_of(&forever)}),
        )
        .await;
    res.assert_detail(400, INVALID);

    let club_invite = clubs["id"].as_i64().unwrap();
    root.delete(&format!("/api/admin/invites/{club_invite}"))
        .await
        .assert_detail(404, "No such invite");
    root.delete("/api/admin/invites/999999")
        .await
        .assert_detail(404, "No such invite");
    accept(&app.user("bob").await, &code_of(&clubs))
        .await
        .assert_status(200);
}

/// `bookclub-server beta-invite` mints a sign-up invite and prints its code and link.
#[sqlx::test]
async fn the_beta_invite_command_prints_a_link(db: PgPool) {
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let name = db.connect_options().get_database().unwrap().to_owned();
    let mut url = url::Url::parse(&base).unwrap();
    url.set_path(&name);
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_bookclub-server"))
            .args(args)
            .env_clear()
            .env("DATABASE_URL", url.as_str())
            .env("DEV_MODE", "true")
            .env("PUBLIC_URL", "https://books.example.com")
            .env("RUST_LOG", "error")
            .output()
            .unwrap()
    };

    let output = run(&["beta-invite", "--uses", "3", "--days", "5"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{stdout}");
    let code = lines[0];
    assert_eq!(lines[1], format!("https://books.example.com/join/{code}"));
    let row: (Option<i64>, Option<i32>, Option<i64>, f64) = sqlx::query_as(
        "SELECT club_id, max_uses, created_by,
                extract(epoch FROM expires_at - now())::float8 / 86400
         FROM club_invites WHERE code = $1",
    )
    .bind(code)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!((row.0, row.1, row.2), (None, Some(3), None));
    assert!((row.3 - 5.0).abs() < 0.01, "{}", row.3);

    let output = run(&["beta-invite"]);
    assert!(output.status.success());
    let code = String::from_utf8(output.stdout).unwrap();
    let code = code.lines().next().unwrap();
    let row: (Option<i32>, bool) = sqlx::query_as(
        "SELECT max_uses, expires_at > now() + interval '29 days' FROM club_invites WHERE code = $1",
    )
    .bind(code)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(row, (Some(1), true));

    let output = run(&["beta-invite", "--uses", "0"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "clap refuses out-of-range flags"
    );

    // The admin list shows no creator for an invite the CLI minted.
    let app = TestApp::new(db).await;
    let listed = app.admin("root").await.get("/api/admin/invites").await;
    listed.assert_status(200);
    let items = listed.json()["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 2);
    for item in items.as_array().unwrap() {
        assert_eq!(item["created_by"], Value::Null, "{item}");
    }
}

/// An invite outlives the accounts that made and used it: `created_by` becomes null and
/// `used_by` leaves the deleted account out, while `use_count` keeps counting the use.
#[sqlx::test]
async fn invites_name_nobody_once_their_accounts_are_gone(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let admin = app.user("admin").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &admin, "admin").await;
    let code = link(&admin, &club).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    for who in [&ada, &bob] {
        accept(who, &code).await.assert_status(200);
    }
    for who in [&admin, &ada] {
        who.delete_json("/api/auth/me", json!({"password": PASSWORD}))
            .await
            .assert_status(204);
    }

    let res = owner
        .get(&format!("/api/clubs/{}/invites", club.slug))
        .await;
    res.assert_status(200);
    let item = res.json()["items"][0].clone();
    assert_eq!(item["code"], code.as_str());
    assert_eq!(item["created_by"], Value::Null);
    assert_eq!(item["used_by"], json!(["bob"]));
    assert_eq!(item["use_count"], 2);
    assert_eq!(item["active"], true);
}
