//! The join page's preview of an invite, `GET /api/invites/{code}` (endpoint-map §3): the
//! inviter while they are in the club, the current pick's book, the member count, the limit,
//! the expiry, and for a signed-in caller whether they are in the club and whether a personal
//! invite is theirs. Never whom a personal invite is for, never anything for a sign-up invite.

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const INVALID: &str = "That invite code is not valid";

async fn link(admin: &TestClient, club: &TestClub) -> String {
    let res = admin
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "link"}),
        )
        .await;
    res.assert_status(201);
    res.json()["code"].as_str().unwrap().to_owned()
}

async fn preview(client: &TestClient, code: &str) -> Value {
    let res = client.get(&format!("/api/invites/{code}")).await;
    res.assert_status(200);
    res.json()
}

async fn befriend(a: &TestClient, b: &TestClient) {
    a.post("/api/friends/requests", json!({"username": b.username()}))
        .await
        .assert_status(200);
    b.post("/api/friends/requests", json!({"username": a.username()}))
        .await
        .assert_status(200);
}

/// Gives `who` an avatar `public_id` (removed when `removed`).
async fn set_avatar(app: &TestApp, who: &TestClient, public_id: &str, removed: bool) {
    app.exec(&format!(
        "WITH avatar AS (
             INSERT INTO media (public_id, kind, source, uploaded_by, removed_at)
             VALUES ('{public_id}', 'avatar', 'upload', {id}, CASE WHEN {removed} THEN now() END)
             RETURNING id
         )
         UPDATE users SET avatar_media_id = (SELECT id FROM avatar) WHERE id = {id}",
        id = who.id()
    ))
    .await;
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
async fn the_preview_names_the_inviter_while_they_are_in_the_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let code = link(&owner, &club).await;
    let anon = app.anon();

    assert_eq!(
        preview(&anon, &code).await["inviter"],
        json!({"username": "owner", "avatar_url": null})
    );
    set_avatar(&app, &owner, "ownerownerownerownerowner1", false).await;
    assert_eq!(
        preview(&anon, &code).await["inviter"],
        json!({"username": "owner", "avatar_url": "/media/ownerownerownerownerowner1/a"})
    );
    set_avatar(&app, &owner, "ownerownerownerownerowner2", true).await;
    assert_eq!(
        preview(&anon, &code).await["inviter"],
        json!({"username": "owner", "avatar_url": null})
    );
}

#[sqlx::test]
async fn an_inviter_who_left_was_deleted_or_is_suspended_is_not_named(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let bea = app.user("bea").await;
    let cleo = app.user("cleo").await;
    let dora = app.user("dora").await;
    let club = app.club(&owner, "Readers").await;
    for admin in [&bea, &cleo, &dora] {
        app.join(&club, admin, "admin").await;
    }
    let left = link(&bea, &club).await;
    let deleted = link(&cleo, &club).await;
    let suspended = link(&dora, &club).await;
    let anon = app.anon();
    for code in [&left, &deleted, &suspended] {
        assert!(preview(&anon, code).await["inviter"].is_object());
    }

    // Leaving keeps the invite valid, but the club no longer has them to speak for it.
    bea.post(&format!("/api/clubs/{}/leave", club.slug), json!({}))
        .await
        .assert_status(204);
    let body = preview(&anon, &left).await;
    assert_eq!(body["inviter"], Value::Null);
    assert_eq!(body["member_count"], 3);

    app.exec(&format!("DELETE FROM users WHERE id = {}", cleo.id()))
        .await;
    assert_eq!(preview(&anon, &deleted).await["inviter"], Value::Null);

    // A suspension revokes the account's invites; one that slipped through names no one.
    app.exec(&format!(
        "UPDATE users SET suspended_at = now() WHERE id = {}",
        dora.id()
    ))
    .await;
    assert_eq!(preview(&anon, &suspended).await["inviter"], Value::Null);
    app.exec(&format!(
        "UPDATE users SET suspended_at = NULL WHERE id = {}",
        dora.id()
    ))
    .await;
    let root = app.admin("root").await;
    root.post("/api/admin/users/dora/suspend", json!({}))
        .await
        .assert_status(200);
    anon.get(&format!("/api/invites/{suspended}"))
        .await
        .assert_detail(404, INVALID);
}

#[sqlx::test]
async fn a_signup_invite_tells_nothing_about_its_creator(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    // The creator is in a club with a pick: none of it shows.
    let club = app.club(&root, "Admins Read Too").await;
    let work = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    root.put(
        &format!("/api/clubs/{}/pick", club.slug),
        json!({"work_id": work.id}),
    )
    .await
    .assert_status(200);
    let res = root.post("/api/admin/invites", json!({})).await;
    res.assert_status(201);
    let code = res.json()["code"].as_str().unwrap().to_owned();

    for client in [app.anon(), root.clone(), app.user("ada").await] {
        let body = preview(&client, &code).await;
        assert_eq!(
            body,
            json!({
                "club": null, "kind": "signup", "personal": false, "inviter": null,
                "pick": null, "member_count": null, "full": false,
                "expires_at": body["expires_at"], "you_are_member": null, "for_you": null,
            })
        );
        assert!(body["expires_at"].is_string());
    }
}

#[sqlx::test]
async fn the_preview_shows_the_current_picks_book_only(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let ada = app.user("ada").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &ada, "member").await;
    let code = link(&owner, &club).await;
    let anon = app.anon();
    assert_eq!(preview(&anon, &code).await["pick"], Value::Null);

    let work = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    app.exec(&format!(
        "WITH cover AS (
             INSERT INTO media (public_id, kind, source, dominant_color)
             VALUES ('circecircecircecircecirce1', 'cover', 'upload', '#7a3b2e')
             RETURNING id
         )
         UPDATE works SET cover_media_id = (SELECT id FROM cover) WHERE id = {}",
        work.id
    ))
    .await;
    owner
        .put(
            &format!("/api/clubs/{}/pick", club.slug),
            json!({"work_id": work.id, "note": "Secret note for members",
                   "meeting_at": "2030-08-22T19:00"}),
        )
        .await
        .assert_status(200);
    app.exec(&format!(
        "INSERT INTO shelf_entries (user_id, work_id, status, rating, take, rating_visibility)
         VALUES ({}, {}, 'finished', 2, 'Hidden take', 'only_me')",
        ada.id(),
        work.id
    ))
    .await;

    let body = preview(&anon, &code).await;
    assert_eq!(
        body["pick"],
        json!({
            "title": "Circe",
            "authors": "Madeline Miller",
            "cover_url": "/media/circecircecircecircecirce1/l",
            "cover_thumb_url": "/media/circecircecircecircecirce1/s",
            "spine_color": "#7a3b2e",
        })
    );
    let text = body.to_string();
    for secret in ["Secret note", "Hidden take", "2030", "OL1W", "\"ada\""] {
        assert!(!text.contains(secret), "{secret} leaks: {text}");
    }

    // A removed cover gives no URLs; an ended pick is no pick.
    app.exec("UPDATE media SET removed_at = now() WHERE kind = 'cover'")
        .await;
    let body = preview(&anon, &code).await;
    assert_eq!(body["pick"]["cover_url"], Value::Null);
    assert_eq!(body["pick"]["spine_color"], Value::Null);
    app.exec("UPDATE club_picks SET ended_at = now()").await;
    assert_eq!(preview(&anon, &code).await["pick"], Value::Null);
}

#[sqlx::test]
async fn the_preview_counts_members_and_says_when_the_club_is_full(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let code = link(&owner, &club).await;
    let anon = app.anon();

    let body = preview(&anon, &code).await;
    assert_eq!(
        (body["member_count"].clone(), body["full"].clone()),
        (json!(1), json!(false))
    );
    fill(&app, &club, 18).await;
    let body = preview(&anon, &code).await;
    assert_eq!(
        (body["member_count"].clone(), body["full"].clone()),
        (json!(19), json!(false))
    );
    fill_more(&app, &club).await;
    let body = preview(&anon, &code).await;
    assert_eq!(
        (body["member_count"].clone(), body["full"].clone()),
        (json!(20), json!(true))
    );

    // The preview only warns: joining is still refused by the limit itself.
    let ada = app.user("ada").await;
    ada.post(&format!("/api/invites/{code}/accept"), json!({}))
        .await
        .assert_detail(409, "This club is full");
}

/// One more member, `last`, so the club reaches 20.
async fn fill_more(app: &TestApp, club: &TestClub) {
    app.exec(&format!(
        "WITH extra AS (
             INSERT INTO users (username, password_hash) VALUES ('last', 'x') RETURNING id
         )
         INSERT INTO club_memberships (club_id, user_id, role)
         SELECT {}, id, 'member' FROM extra",
        club.id
    ))
    .await;
}

#[sqlx::test]
async fn a_session_adds_whether_the_caller_is_in_the_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let ada = app.user("ada").await;
    let club = app.club(&owner, "Readers").await;
    let code = link(&owner, &club).await;

    let anon = preview(&app.anon(), &code).await;
    assert_eq!(
        (anon["you_are_member"].clone(), anon["for_you"].clone()),
        (Value::Null, Value::Null)
    );
    let outsider = preview(&ada, &code).await;
    assert_eq!(outsider["you_are_member"], false);
    assert_eq!(outsider["for_you"], Value::Null);
    let member = preview(&owner, &code).await;
    assert_eq!(member["you_are_member"], true);
    assert_eq!(member["for_you"], Value::Null);

    // Apart from those two fields, everyone sees the same.
    for mut body in [outsider, member] {
        body["you_are_member"] = Value::Null;
        assert_eq!(body, anon);
    }
}

#[sqlx::test]
async fn a_personal_preview_never_names_whom_it_is_for(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bobbington").await;
    let carol = app.user("carol").await;
    let club = app.club(&ada, "Readers").await;
    befriend(&ada, &bob).await;
    let res = ada
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "friend", "username": "bobbington"}),
        )
        .await;
    res.assert_status(201);
    let code = res.json()["code"].as_str().unwrap().to_owned();

    let anon = preview(&app.anon(), &code).await;
    assert_eq!(anon["personal"], true);
    assert_eq!(
        anon["inviter"],
        json!({"username": "ada", "avatar_url": null})
    );
    assert_eq!(anon["for_you"], Value::Null);
    let other = preview(&carol, &code).await;
    assert_eq!(other["for_you"], false);
    assert_eq!(other["you_are_member"], false);
    let invitee = preview(&bob, &code).await;
    assert_eq!(invitee["for_you"], true);
    assert_eq!(invitee["you_are_member"], false);
    for body in [&anon, &other, &invitee] {
        let text = body.to_string();
        assert!(!text.contains("bobbington"), "the invitee leaks: {text}");
    }
}
