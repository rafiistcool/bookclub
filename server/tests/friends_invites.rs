//! Friend invites into clubs (FRIENDS, #57; docs/phase3/work-plan.md §4.6, §6 answers 22 and
//! 24): a club admin invites a friend with a single-use code only that friend can redeem, for 30
//! days; the friend joins through the usual join page.

mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const INVALID: &str = "That invite code is not valid";
const NO_FRIEND: &str = "No friend with that name";

async fn befriend(a: &TestClient, b: &TestClient) {
    a.post("/api/friends/requests", json!({"username": b.username()}))
        .await
        .assert_status(200);
    b.post("/api/friends/requests", json!({"username": a.username()}))
        .await
        .assert_status(200);
}

async fn invite_friend(admin: &TestClient, club: &TestClub, friend: &str) -> TestResponse {
    admin
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "friend", "username": friend}),
        )
        .await
}

/// The new (or open) friend invite; panics unless 201.
async fn invited(admin: &TestClient, club: &TestClub, friend: &str) -> Value {
    let res = invite_friend(admin, club, friend).await;
    res.assert_status(201);
    res.json()
}

fn code_of(invite: &Value) -> String {
    invite["code"].as_str().unwrap().to_owned()
}

async fn accept(client: &TestClient, code: &str) -> TestResponse {
    client
        .post(&format!("/api/invites/{code}/accept"), json!({}))
        .await
}

async fn club_invites(who: &TestClient) -> Value {
    let res = who.get("/api/friends").await;
    res.assert_status(200);
    res.json()["club_invites"].clone()
}

async fn invites_of(admin: &TestClient, club: &TestClub) -> Vec<Value> {
    let res = admin
        .get(&format!("/api/clubs/{}/invites", club.slug))
        .await;
    res.assert_status(200);
    res.json()["items"].as_array().unwrap().clone()
}

fn days_from_now(timestamp: &Value) -> f64 {
    let at = OffsetDateTime::parse(timestamp.as_str().unwrap(), &Rfc3339).unwrap();
    (at - OffsetDateTime::now_utc()).as_seconds_f64() / 86_400.0
}

async fn is_active(app: &TestApp, code: &str) -> bool {
    app.scalar_i64(&format!(
        "SELECT count(*) FROM club_invites
         WHERE code = '{code}' AND revoked_at IS NULL AND expires_at > now()
           AND use_count < max_uses"
    ))
    .await
        == 1
}

/// The acceptance's end to end: the admin invites a friend, the friend sees the invitation,
/// previews the code, joins, and is a member invited by the admin.
#[sqlx::test]
async fn an_admin_invites_a_friend_who_joins(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let club = app.club(&ada, "Readers").await;
    befriend(&ada, &bob).await;

    let invite = invited(&ada, &club, " Bob ").await;
    let code = code_of(&invite);
    assert_eq!(
        invite,
        json!({
            "id": invite["id"],
            "code": code,
            "url": format!("http://localhost:5173/join/{code}"),
            "created_by": "ada",
            "created_at": invite["created_at"],
            "expires_at": invite["expires_at"],
            "max_uses": 1,
            "use_count": 0,
            "used_by": [],
            "revoked": false,
            "active": true,
            "personal": true,
            "invitee": "bob",
        })
    );
    assert!((days_from_now(&invite["expires_at"]) - 30.0).abs() < 0.01);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT invitee_id FROM club_invites WHERE code = '{code}'"
        ))
        .await,
        bob.id()
    );

    assert_eq!(
        club_invites(&bob).await,
        json!([{
            "id": invite["id"],
            "club": {"slug": club.slug, "name": "Readers"},
            "invited_by": "ada",
            "code": code,
            "created_at": invite["created_at"],
            "expires_at": invite["expires_at"],
        }])
    );
    assert_eq!(club_invites(&ada).await, json!([]));
    let preview = bob.get(&format!("/api/invites/{code}")).await;
    preview.assert_status(200);
    assert_eq!(
        preview.json(),
        json!({"club": {"slug": club.slug, "name": "Readers"}, "kind": "club"})
    );
    let res = accept(&bob, &code).await;
    res.assert_status(200);
    assert_eq!(res.json()["slug"], club.slug.as_str());
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT invited_by FROM club_memberships WHERE club_id = {} AND user_id = {}",
            club.id,
            bob.id()
        ))
        .await,
        ada.id()
    );
    assert_eq!(club_invites(&bob).await, json!([]));
    let listed = invites_of(&ada, &club).await;
    assert_eq!(listed[0]["used_by"], json!(["bob"]));
    assert_eq!(listed[0]["active"], false);
}

/// Club admins only (answer 22): a plain member gets 403, like for every invite.
#[sqlx::test]
async fn only_club_admins_invite_friends(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &bob, "member").await;
    befriend(&bob, &cy).await;
    invite_friend(&bob, &club, "cy")
        .await
        .assert_detail(403, "Only club admins can do that");
    let admin = app.user("dee").await;
    app.join(&club, &admin, "admin").await;
    befriend(&admin, &cy).await;
    invited(&admin, &club, "cy").await;
    assert_eq!(app.scalar_i64("SELECT count(*) FROM club_invites").await, 1);
}

/// A stranger, an unknown name, the caller, both sides of a pending request and a blocked
/// friend (either way) all get the same 404: the answer never tells them apart.
#[sqlx::test]
async fn an_invite_needs_a_friend(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    app.user("stranger").await;
    let asked = app.user("asked").await;
    app.user("requested").await;
    let blocked = app.user("blocked").await;
    let blocker = app.user("blocker").await;
    asked
        .post("/api/friends/requests", json!({"username": "ada"}))
        .await
        .assert_status(200);
    ada.post("/api/friends/requests", json!({"username": "requested"}))
        .await
        .assert_status(200);
    befriend(&ada, &blocked).await;
    ada.put("/api/friends/blocks/blocked", json!({}))
        .await
        .assert_status(200);
    befriend(&ada, &blocker).await;
    blocker
        .put("/api/friends/blocks/ada", json!({}))
        .await
        .assert_status(200);
    for name in [
        "stranger",
        "nobody_here",
        "ada",
        "asked",
        "requested",
        "blocked",
        "blocker",
    ] {
        invite_friend(&ada, &club, name)
            .await
            .assert_detail(404, NO_FRIEND);
    }
    // The blocked account's attempt, from its own club, answers the same.
    let theirs = app.club(&blocked, "Theirs").await;
    invite_friend(&blocked, &theirs, "ada")
        .await
        .assert_detail(404, NO_FRIEND);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM club_invites").await, 0);
}

/// The body: a friend invite needs `username`, the other kinds refuse one; the expiry is 1 to
/// 30 days.
#[sqlx::test]
async fn a_friend_invite_names_its_friend(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let club = app.club(&ada, "Readers").await;
    befriend(&ada, &bob).await;
    let path = format!("/api/clubs/{}/invites", club.slug);
    for (body, detail) in [
        (json!({"kind": "friend"}), "Field required"),
        (
            json!({"kind": "friend", "username": null}),
            "Field required",
        ),
        (
            json!({"kind": "link", "username": "bob"}),
            "Only a friend invite names a person",
        ),
        (
            json!({"kind": "single_use", "username": "bob"}),
            "Only a friend invite names a person",
        ),
        (
            json!({"kind": "friend", "username": "bob", "expires_in_days": 31}),
            "Input should be less than or equal to 30",
        ),
        (
            json!({"kind": "friend", "username": "bob\u{0}"}),
            "Invalid value",
        ),
    ] {
        ada.post(&path, body).await.assert_detail(400, detail);
    }
    let short = ada
        .post(
            &path,
            json!({"kind": "friend", "username": "bob", "expires_in_days": 7}),
        )
        .await;
    short.assert_status(201);
    assert!((days_from_now(&short.json()["expires_at"]) - 7.0).abs() < 0.01);
    // Links and single-use codes are as before, never personal.
    let link = ada.post(&path, json!({"kind": "link"})).await.json();
    assert_eq!(
        (&link["personal"], &link["invitee"]),
        (&json!(false), &Value::Null)
    );
}

/// A friend who is already in the club gets 409.
#[sqlx::test]
async fn a_member_needs_no_invite(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let club = app.club(&ada, "Readers").await;
    befriend(&ada, &bob).await;
    app.join(&club, &bob, "member").await;
    invite_friend(&ada, &club, "bob")
        .await
        .assert_detail(409, "That friend is already in this club");
}

/// A second invite for the same friend answers with the admin's open one; once that is used
/// up, revoked or expired, a new one is made. A co-admin's invite is their own (answer 24: the
/// first would tell them whom the other admin is friends with).
#[sqlx::test]
async fn a_second_invite_returns_the_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &cy, "admin").await;
    befriend(&ada, &bob).await;
    befriend(&cy, &bob).await;
    let first = invited(&ada, &club, "bob").await;
    let again = invited(&ada, &club, "bob").await;
    assert_eq!(again, first);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM club_invites").await, 1);
    let theirs = invited(&cy, &club, "bob").await;
    assert_ne!(theirs["id"], first["id"]);
    assert_eq!(theirs["invitee"], "bob");
    // Revoked: a new one.
    ada.delete(&format!("/api/clubs/{}/invites/{}", club.slug, first["id"]))
        .await
        .assert_status(204);
    let second = invited(&ada, &club, "bob").await;
    assert_ne!(second["id"], first["id"]);
    // Expired: a new one.
    app.exec(&format!(
        "UPDATE club_invites SET expires_at = now() - interval '1 second' WHERE id = {}",
        second["id"]
    ))
    .await;
    let third = invited(&ada, &club, "bob").await;
    assert_ne!(third["id"], second["id"]);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM club_invites").await, 4);
}

/// Only the friend can redeem it: another account's accept and a sign-up with the code are 400
/// `That invite code is not valid`, and the invite stays unused for the friend.
#[sqlx::test]
async fn a_friend_invite_is_for_that_friend_only(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let mallory = app.user("mallory").await;
    let club = app.club(&ada, "Readers").await;
    befriend(&ada, &bob).await;
    let code = code_of(&invited(&ada, &club, "bob").await);

    accept(&mallory, &code).await.assert_detail(400, INVALID);
    app.anon()
        .post(
            "/api/auth/register",
            json!({"username": "newcomer", "email": "new@example.test",
                   "password": PASSWORD, "invite_code": code}),
        )
        .await
        .assert_detail(400, INVALID);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE username = 'newcomer'")
            .await,
        0
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_invite_uses")
            .await,
        0
    );
    assert!(is_active(&app, &code).await);
    accept(&bob, &code).await.assert_status(200);
}

/// The 20-member cap holds for a friend invite like for every other.
#[sqlx::test]
async fn the_member_cap_holds(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let club = app.club(&ada, "Readers").await;
    befriend(&ada, &bob).await;
    let code = code_of(&invited(&ada, &club, "bob").await);
    app.exec(&format!(
        "WITH extra AS (
             INSERT INTO users (username, password_hash)
             SELECT 'filler' || i, 'x' FROM generate_series(1, 19) AS i
             RETURNING id
         )
         INSERT INTO club_memberships (club_id, user_id, role)
         SELECT {}, id, 'member' FROM extra",
        club.id
    ))
    .await;
    accept(&bob, &code)
        .await
        .assert_detail(409, "This club is full");
    assert!(
        is_active(&app, &code).await,
        "the refused join used nothing"
    );
}

/// Declining, unfriending, blocking and a removal from the club each revoke it.
#[sqlx::test]
async fn decline_unfriend_block_and_removal_revoke(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    let club = app.club(&ada, "Readers").await;
    befriend(&ada, &bob).await;

    // Declined by the friend; nobody else can decline it.
    let invite = invited(&ada, &club, "bob").await;
    let path = format!("/api/friends/club-invites/{}", invite["id"]);
    for who in [&ada, &cy] {
        who.delete(&path).await.assert_detail(404, "No such invite");
    }
    let res = bob.delete(&path).await;
    res.assert_status(200);
    assert_eq!(res.json()["club_invites"], json!([]));
    assert!(!is_active(&app, &code_of(&invite)).await);
    bob.delete(&path).await.assert_status(200);
    accept(&bob, &code_of(&invite))
        .await
        .assert_detail(400, INVALID);
    let link = ada
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "link"}),
        )
        .await
        .json();
    bob.delete(&format!("/api/friends/club-invites/{}", link["id"]))
        .await
        .assert_detail(404, "No such invite");

    // Unfriended, by either side.
    for remover in [&ada, &bob] {
        let code = code_of(&invited(&ada, &club, "bob").await);
        let other = if remover.id() == ada.id() {
            "bob"
        } else {
            "ada"
        };
        remover
            .delete(&format!("/api/friends/list/{other}"))
            .await
            .assert_status(200);
        assert!(!is_active(&app, &code).await);
        accept(&bob, &code).await.assert_detail(400, INVALID);
        befriend(&ada, &bob).await;
    }

    // Blocked.
    let code = code_of(&invited(&ada, &club, "bob").await);
    ada.put("/api/friends/blocks/bob", json!({}))
        .await
        .assert_status(200);
    assert!(!is_active(&app, &code).await);
    ada.delete("/api/friends/blocks/bob")
        .await
        .assert_status(200);
    befriend(&ada, &bob).await;

    // Removed from the club after joining through a link: the open friend invite made for them
    // goes too (clubs::members::revoke_known_invites).
    let code = code_of(&invited(&ada, &club, "bob").await);
    accept(&bob, link["code"].as_str().unwrap())
        .await
        .assert_status(200);
    ada.delete(&format!("/api/clubs/{}/members/bob", club.slug))
        .await
        .assert_status(204);
    assert!(!is_active(&app, &code).await);
}

/// A friend's decline is not lasting: the admin may invite again, as a declined request may be
/// sent again (§6 answer 19). 20 friend invites per account a day bound both (FRIENDS security
/// review), unknown names and repeats of an open invite included; the other kinds stay free.
#[sqlx::test]
async fn twenty_friend_invites_a_day(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let club = app.club(&ada, "Readers").await;
    befriend(&ada, &bob).await;
    for _ in 0..10 {
        let invite = invited(&ada, &club, "bob").await;
        bob.delete(&format!("/api/friends/club-invites/{}", invite["id"]))
            .await
            .assert_status(200);
    }
    let open = invited(&ada, &club, "bob").await;
    for _ in 0..4 {
        assert_eq!(invited(&ada, &club, "bob").await["id"], open["id"]);
    }
    for _ in 0..5 {
        invite_friend(&ada, &club, "nobody_here")
            .await
            .assert_detail(404, NO_FRIEND);
    }
    let res = invite_friend(&ada, &club, "bob").await;
    res.assert_detail(429, "Slow down a little. Try again later.");
    let wait: u64 = res.header("retry-after").unwrap().parse().unwrap();
    assert!((86_000..=86_400).contains(&wait), "{wait}");
    ada.post(
        &format!("/api/clubs/{}/invites", club.slug),
        json!({"kind": "single_use"}),
    )
    .await
    .assert_status(201);
    assert_eq!(club_invites(&bob).await.as_array().unwrap().len(), 1);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_invites WHERE invitee_id IS NOT NULL")
            .await,
        11
    );
}

/// 30 days: a 21-day-old invite still works, a 31-day-old one does not and is no longer listed.
#[sqlx::test]
async fn a_friend_invite_lasts_thirty_days(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let club = app.club(&ada, "Readers").await;
    let old_club = app.club(&ada, "Old").await;
    befriend(&ada, &bob).await;
    let fresh = code_of(&invited(&ada, &club, "bob").await);
    let stale = code_of(&invited(&ada, &old_club, "bob").await);
    app.exec(&format!(
        "UPDATE club_invites SET created_at = created_at - interval '21 days',
                                 expires_at = expires_at - interval '21 days'
         WHERE code = '{fresh}';
         UPDATE club_invites SET created_at = created_at - interval '31 days',
                                 expires_at = expires_at - interval '31 days'
         WHERE code = '{stale}'"
    ))
    .await;
    let listed = club_invites(&bob).await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["code"], fresh.as_str());
    accept(&bob, &stale).await.assert_detail(400, INVALID);
    accept(&bob, &fresh).await.assert_status(200);
}

/// Answer 24: the invitee's name is shown to the invite's creator only; another admin of the
/// club sees a personal invite without the name, before and after the friend joins with it.
#[sqlx::test]
async fn only_the_creator_sees_whom_an_invite_is_for(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &cy, "admin").await;
    befriend(&ada, &bob).await;
    let invite = invited(&ada, &club, "bob").await;
    let mine = invites_of(&ada, &club).await;
    assert_eq!(
        (&mine[0]["personal"], &mine[0]["invitee"]),
        (&json!(true), &json!("bob"))
    );
    let theirs = invites_of(&cy, &club).await;
    assert_eq!(theirs[0]["id"], invite["id"]);
    assert_eq!(
        (&theirs[0]["personal"], &theirs[0]["invitee"]),
        (&json!(true), &Value::Null)
    );
    assert_eq!(theirs[0]["created_by"], "ada");
    assert!(!theirs.iter().any(|i| i.to_string().contains("\"bob\"")));

    // Once used, the invite still names the friend to its creator alone: `used_by` would
    // otherwise tell the co-admin whom it was for.
    accept(&bob, &code_of(&invite)).await.assert_status(200);
    let mine = invites_of(&ada, &club).await;
    assert_eq!(
        (
            &mine[0]["invitee"],
            &mine[0]["used_by"],
            &mine[0]["use_count"]
        ),
        (&json!("bob"), &json!(["bob"]), &json!(1))
    );
    let theirs = invites_of(&cy, &club).await;
    assert_eq!(theirs[0]["id"], invite["id"]);
    assert_eq!(
        (
            &theirs[0]["personal"],
            &theirs[0]["invitee"],
            &theirs[0]["used_by"],
            &theirs[0]["use_count"],
            &theirs[0]["active"]
        ),
        (
            &json!(true),
            &Value::Null,
            &json!([]),
            &json!(1),
            &json!(false)
        )
    );
    assert!(!theirs.iter().any(|i| i.to_string().contains("\"bob\"")));
    // An ordinary invite still lists who used it, for every admin.
    let open = cy
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "single_use"}),
        )
        .await;
    open.assert_status(201);
    let dan = app.user("dan").await;
    accept(&dan, &code_of(&open.json()))
        .await
        .assert_status(200);
    for admin in [&ada, &cy] {
        let listed = invites_of(admin, &club).await;
        assert_eq!(listed[0]["used_by"], json!(["dan"]));
    }
}

/// The invite reads the friendship under a lock (`FOR SHARE`, after the club's): an unfriend
/// that holds the friendship's row makes the invite wait, and once it commits the invite finds
/// no friend. A plain read would have made an invite the unfriend never revoked.
#[sqlx::test]
async fn an_invite_waits_for_an_unfriend_in_flight(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let club = app.club(&ada, "Readers").await;
    befriend(&ada, &bob).await;

    let mut unfriend = app.db.begin().await.unwrap();
    sqlx::query(
        "SELECT 1 FROM friendships
         WHERE (requester_id, addressee_id) IN (($1, $2), ($2, $1)) FOR UPDATE",
    )
    .bind(ada.id())
    .bind(bob.id())
    .execute(&mut *unfriend)
    .await
    .unwrap();
    let task = tokio::spawn({
        let ada = ada.clone();
        let club = club.clone();
        async move { invite_friend(&ada, &club, "bob").await }
    });
    wait_for_a_lock(&app).await;
    sqlx::query(
        "DELETE FROM friendships WHERE (requester_id, addressee_id) IN (($1, $2), ($2, $1))",
    )
    .bind(ada.id())
    .bind(bob.id())
    .execute(&mut *unfriend)
    .await
    .unwrap();
    unfriend.commit().await.unwrap();
    task.await.unwrap().assert_detail(404, NO_FRIEND);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM club_invites").await, 0);
}

/// Waits until a session of the test database waits for a lock.
async fn wait_for_a_lock(app: &TestApp) {
    for _ in 0..500 {
        let waiting = app
            .scalar_i64(
                "SELECT count(*) FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event_type = 'Lock'",
            )
            .await;
        if waiting >= 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the invite never waited for a lock");
}
