//! The member list and the membership writes: role changes, removal, leaving, the ownership
//! transfer, and the owner guarantees under concurrency (endpoint-map §3,
//! docs/phase1/conventions.md §5.3).

mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;

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
    panic!("{n} requests never waited for the club lock");
}

async fn owners(app: &TestApp, club: &TestClub) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT u.username FROM club_memberships m JOIN users u ON u.id = m.user_id
         WHERE m.club_id = $1 AND m.role = 'owner'",
    )
    .bind(club.id)
    .fetch_all(&app.db)
    .await
    .unwrap()
}

async fn role_of(app: &TestApp, club: &TestClub, who: &TestClient) -> Option<String> {
    sqlx::query_scalar("SELECT role FROM club_memberships WHERE club_id = $1 AND user_id = $2")
        .bind(club.id)
        .bind(who.id())
        .fetch_optional(&app.db)
        .await
        .unwrap()
}

/// Puts `work` on `who`'s shelf as currently reading at `position`.
async fn reading(app: &TestApp, who: &TestClient, work: &TestWork, position: i32) {
    app.exec(&format!(
        "INSERT INTO shelf_entries (user_id, work_id, status, position)
         VALUES ({}, {}, 'currently_reading', {position})",
        who.id(),
        work.id
    ))
    .await;
}

fn usernames(list: &Value) -> Vec<&str> {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["username"].as_str().unwrap())
        .collect()
}

/// parity: test_members.py::test_members_hide_self_and_preview_reading
///
/// Reasoned change: the list is a club roster now and includes the caller (`you: true`), so
/// the role and transfer UI can show them. The reading preview is kept: count, title and the
/// Open Library key of the work.
#[sqlx::test]
async fn members_list_includes_the_caller_and_previews_reading(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    reading(&app, &ada, &circe, 0).await;

    let res = grace
        .get(&format!("/api/clubs/{}/members", club.slug))
        .await;
    res.assert_status(200);
    let list = res.json();
    assert_eq!(usernames(&list), ["ada", "grace"]);
    let ada_row = &list["items"][0];
    assert_eq!(ada_row["you"], false);
    assert_eq!(ada_row["currently_reading_count"], 1);
    assert_eq!(ada_row["currently_reading_preview"][0]["title"], "Circe");
    assert!(
        ada_row["currently_reading_preview"][0]["ol_work_key"]
            .as_str()
            .unwrap()
            .starts_with("/works/")
    );
    let grace_row = &list["items"][1];
    assert_eq!(grace_row["you"], true);
    assert_eq!(grace_row["currently_reading_count"], 0);
    assert_eq!(grace_row["currently_reading_preview"], json!([]));
}

#[sqlx::test]
async fn members_are_ordered_by_role_then_name(db: PgPool) {
    let app = TestApp::new(db).await;
    let zed = app.user("zed").await;
    let club = app.club(&zed, "Readers").await;
    for (name, role) in [
        ("bob", "admin"),
        ("cy", "member"),
        ("amy", "admin"),
        ("al", "member"),
    ] {
        let who = app.user(name).await;
        app.join(&club, &who, role).await;
    }
    // Someone in another club only is not listed.
    let other = app.user("aaron").await;
    app.club(&other, "Other").await;

    let list = zed
        .get(&format!("/api/clubs/{}/members", club.slug))
        .await
        .json();
    assert_eq!(usernames(&list), ["zed", "amy", "bob", "al", "cy"]);
    let roles: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["owner", "admin", "admin", "member", "member"]);
    let first = &list["items"][0];
    assert_eq!(first["you"], true);
    assert_eq!(first["avatar_url"], Value::Null);
    assert!(first["joined_at"].as_str().unwrap().ends_with('Z'));
    let keys: Vec<&String> = first.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "avatar_url",
            "currently_reading_count",
            "currently_reading_preview",
            "joined_at",
            "role",
            "username",
            "you"
        ]
    );
}

/// The preview holds at most three books in shelf order (position, id), uses the shelf
/// edition's title, and counts only currently-reading entries.
#[sqlx::test]
async fn the_reading_preview_is_three_books_in_shelf_order(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &bob, "member").await;
    let mut works = Vec::new();
    for title in ["One", "Two", "Three", "Four", "Later"] {
        works.push(app.work(title).await);
    }
    reading(&app, &bob, &works[3], 3).await;
    reading(&app, &bob, &works[1], 1).await;
    reading(&app, &bob, &works[0], 0).await;
    reading(&app, &bob, &works[2], 1).await;
    app.exec(&format!(
        "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ({}, {}, 'want_to_read')",
        bob.id(),
        works[4].id
    ))
    .await;
    let edition = app.edition(&works[0], "9780316556347").await;
    app.exec(&format!(
        "UPDATE editions SET title = 'One (paperback)' WHERE id = {0};
         UPDATE shelf_entries SET edition_id = {0} WHERE work_id = {1}",
        edition.id, works[0].id
    ))
    .await;

    let list = ada
        .get(&format!("/api/clubs/{}/members", club.slug))
        .await
        .json();
    let bob_row = &list["items"][1];
    assert_eq!(bob_row["username"], "bob");
    assert_eq!(bob_row["currently_reading_count"], 4);
    let titles: Vec<&str> = bob_row["currently_reading_preview"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["title"].as_str().unwrap())
        .collect();
    // Two and Three share position 1; the entry id breaks the tie.
    assert_eq!(titles, ["One (paperback)", "Two", "Three"]);
    assert_eq!(bob_row["currently_reading_preview"][0]["id"], works[0].id);
}

#[sqlx::test]
async fn the_owner_changes_roles(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let bob = app.user("bob").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &bob, "member").await;
    let outsider = app.user("outsider").await;
    let path = |name: &str| format!("/api/clubs/{}/members/{name}", club.slug);

    let res = owner.patch(&path("bob"), json!({"role": "admin"})).await;
    res.assert_status(200);
    let out = res.json();
    assert_eq!(out["username"], "bob");
    assert_eq!(out["role"], "admin");
    assert_eq!(out["you"], false);
    assert_eq!(role_of(&app, &club, &bob).await.as_deref(), Some("admin"));
    // The name is normalised like every username lookup.
    owner
        .patch(&path("%20BOB%20"), json!({"role": "member"}))
        .await
        .assert_status(200);
    assert_eq!(role_of(&app, &club, &bob).await.as_deref(), Some("member"));

    owner
        .patch(&path("bob"), json!({"role": "owner"}))
        .await
        .assert_detail(400, "Use transfer to change the owner");
    owner
        .patch(&path("owner"), json!({"role": "member"}))
        .await
        .assert_detail(400, "Use transfer to change the owner");
    owner
        .patch(&path("bob"), json!({"role": "boss"}))
        .await
        .assert_detail(400, "Invalid value");
    owner
        .patch(&path("nobody"), json!({"role": "admin"}))
        .await
        .assert_detail(404, "No member with that name");
    owner
        .patch(&path(outsider.username()), json!({"role": "admin"}))
        .await
        .assert_detail(404, "No member with that name");
    assert_eq!(owners(&app, &club).await, ["owner"]);
    assert_eq!(role_of(&app, &club, &bob).await.as_deref(), Some("member"));
}

#[sqlx::test]
async fn admins_remove_members_and_the_owner_removes_admins(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let admin = app.user("admin").await;
    let admin2 = app.user("admin2").await;
    let member = app.user("member").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &admin, "admin").await;
    app.join(&club, &admin2, "admin").await;
    app.join(&club, &member, "member").await;
    let path = |name: &str| format!("/api/clubs/{}/members/{name}", club.slug);

    admin
        .delete(&path("admin2"))
        .await
        .assert_detail(403, "Only the club owner can do that");
    admin
        .delete(&path("owner"))
        .await
        .assert_detail(403, "Only the club owner can do that");
    admin
        .delete(&path("admin"))
        .await
        .assert_detail(400, "Use leave to leave the club");
    owner
        .delete(&path("owner"))
        .await
        .assert_detail(400, "Use leave to leave the club");
    admin
        .delete(&path("nobody"))
        .await
        .assert_detail(404, "No member with that name");
    member
        .delete(&path("admin"))
        .await
        .assert_detail(403, "Only club admins can do that");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM club_memberships WHERE club_id = {}",
            club.id
        ))
        .await,
        4
    );

    admin.delete(&path("member")).await.assert_status(204);
    owner.delete(&path("admin2")).await.assert_status(204);
    assert_eq!(role_of(&app, &club, &member).await, None);
    assert_eq!(role_of(&app, &club, &admin2).await, None);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM club_removals WHERE club_id = {} AND user_id IN ({}, {})",
            club.id,
            member.id(),
            admin2.id()
        ))
        .await,
        2
    );
    member
        .get(&format!("/api/clubs/{}", club.slug))
        .await
        .assert_detail(404, "No such club");
    admin
        .delete(&path("member"))
        .await
        .assert_detail(404, "No member with that name");
}

#[sqlx::test]
async fn members_leave_but_the_owner_transfers_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let admin = app.user("admin").await;
    let member = app.user("member").await;
    let club = app.club(&owner, "Readers").await;
    let solo = app.club(&owner, "Solo").await;
    app.join(&club, &admin, "admin").await;
    app.join(&club, &member, "member").await;
    let leave = |slug: &str| format!("/api/clubs/{slug}/leave");

    member
        .post(&leave(&club.slug), json!({}))
        .await
        .assert_status(204);
    admin
        .request(
            axum::http::Method::POST,
            &leave(&club.slug),
            None,
            Vec::new(),
        )
        .await
        .assert_status(204);
    assert_eq!(role_of(&app, &club, &member).await, None);
    assert_eq!(role_of(&app, &club, &admin).await, None);
    // Leaving is not a removal.
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_removals").await,
        0
    );
    member
        .post(&leave(&club.slug), json!({}))
        .await
        .assert_detail(404, "No such club");
    for slug in [&club.slug, &solo.slug] {
        owner
            .post(&leave(slug), json!({}))
            .await
            .assert_detail(409, "Transfer ownership before leaving");
    }
    assert_eq!(owners(&app, &club).await, ["owner"]);
}

#[sqlx::test]
async fn the_owner_transfers_the_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let admin = app.user("admin").await;
    let member = app.user("member").await;
    let outsider = app.user("outsider").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &admin, "admin").await;
    app.join(&club, &member, "member").await;
    let path = format!("/api/clubs/{}/transfer", club.slug);

    for name in ["nobody", outsider.username()] {
        owner
            .post(&path, json!({"username": name}))
            .await
            .assert_detail(404, "No member with that name");
    }
    // A refused transfer rolled the demotion back.
    assert_eq!(owners(&app, &club).await, ["owner"]);
    admin
        .post(&path, json!({"username": "admin"}))
        .await
        .assert_detail(403, "Only the club owner can do that");

    let res = owner.post(&path, json!({"username": " Member "})).await;
    res.assert_status(200);
    assert_eq!(res.json()["role"], "admin");
    assert_eq!(res.json()["slug"], club.slug);
    assert_eq!(owners(&app, &club).await, ["member"]);
    assert_eq!(role_of(&app, &club, &owner).await.as_deref(), Some("admin"));
    // The roles follow at once.
    owner
        .delete(&format!("/api/clubs/{}", club.slug))
        .await
        .assert_detail(403, "Only the club owner can do that");
    member
        .post(&path, json!({"username": "owner"}))
        .await
        .assert_status(200);
    assert_eq!(owners(&app, &club).await, ["owner"]);
    // Transferring to oneself changes nothing.
    let res = owner.post(&path, json!({"username": "owner"})).await;
    res.assert_status(200);
    assert_eq!(res.json()["role"], "owner");
    assert_eq!(owners(&app, &club).await, ["owner"]);
}

/// Sends `first`, waits until it queues behind the held club lock, then `second`, and releases
/// the lock: the two run in that order, each under the lock.
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

/// One succession scenario: `owner` is about to hand the club over to `heir`, and `admin`
/// could remove the heir. Usernames end in `_<tag>`, so one test runs several scenarios.
struct Succession {
    club: TestClub,
    owner: TestClient,
    heir: TestClient,
    admin: TestClient,
}

impl Succession {
    async fn new(app: &TestApp, tag: &str) -> Self {
        let owner = app.user(&format!("owner_{tag}")).await;
        let heir = app.user(&format!("heir_{tag}")).await;
        let admin = app.user(&format!("admin_{tag}")).await;
        let club = app.club(&owner, "Readers").await;
        app.join(&club, &heir, "member").await;
        app.join(&club, &admin, "admin").await;
        Succession {
            club,
            owner,
            heir,
            admin,
        }
    }

    fn transfer(&self) -> impl Future<Output = TestResponse> + Send + 'static {
        let owner = self.owner.clone();
        let path = format!("/api/clubs/{}/transfer", self.club.slug);
        let body = json!({"username": self.heir.username()});
        async move { owner.post(&path, body).await }
    }

    fn remove_heir(&self, by: &TestClient) -> impl Future<Output = TestResponse> + Send + 'static {
        let by = by.clone();
        let path = format!(
            "/api/clubs/{}/members/{}",
            self.club.slug,
            self.heir.username()
        );
        async move { by.delete(&path).await }
    }

    fn heir_leaves(&self) -> impl Future<Output = TestResponse> + Send + 'static {
        let heir = self.heir.clone();
        let path = format!("/api/clubs/{}/leave", self.club.slug);
        async move { heir.post(&path, json!({})).await }
    }

    async fn owners(&self, app: &TestApp) -> Vec<String> {
        owners(app, &self.club).await
    }
}

/// A transfer concurrent with the removal of the new owner ends with exactly one owner, in
/// either order. The removal re-reads both roles under the club lock.
#[sqlx::test]
async fn a_transfer_racing_the_removal_of_the_heir_leaves_one_owner(db: PgPool) {
    let app = TestApp::new(db).await;

    // Transfer first: the heir is the owner when the admin's removal runs, so it is refused.
    let s = Succession::new(&app, "a").await;
    let (transfer, removal) = in_order(&app, &s.club, s.transfer(), s.remove_heir(&s.admin)).await;
    transfer.assert_status(200);
    removal.assert_detail(403, "Only the club owner can do that");
    assert_eq!(s.owners(&app).await, ["heir_a"]);

    // The old owner's extractor still read "owner", but the removal runs on the new roles.
    let s = Succession::new(&app, "b").await;
    let (transfer, removal) = in_order(&app, &s.club, s.transfer(), s.remove_heir(&s.owner)).await;
    transfer.assert_status(200);
    removal.assert_detail(403, "Only the club owner can do that");
    assert_eq!(s.owners(&app).await, ["heir_b"]);

    // Removal first: the transfer finds no heir and rolls its demotion back.
    let s = Succession::new(&app, "c").await;
    let (removal, transfer) = in_order(&app, &s.club, s.remove_heir(&s.admin), s.transfer()).await;
    removal.assert_status(204);
    transfer.assert_detail(404, "No member with that name");
    assert_eq!(s.owners(&app).await, ["owner_c"]);
}

/// A transfer concurrent with the new owner leaving ends with exactly one owner, in either
/// order: the leave's guard (`role <> 'owner'`) refuses an owner.
#[sqlx::test]
async fn a_transfer_racing_the_heir_leaving_leaves_one_owner(db: PgPool) {
    let app = TestApp::new(db).await;

    let s = Succession::new(&app, "a").await;
    let (transfer, left) = in_order(&app, &s.club, s.transfer(), s.heir_leaves()).await;
    transfer.assert_status(200);
    left.assert_detail(409, "Transfer ownership before leaving");
    assert_eq!(s.owners(&app).await, ["heir_a"]);

    let s = Succession::new(&app, "b").await;
    let (left, transfer) = in_order(&app, &s.club, s.heir_leaves(), s.transfer()).await;
    left.assert_status(204);
    transfer.assert_detail(404, "No member with that name");
    assert_eq!(s.owners(&app).await, ["owner_b"]);
}

fn delete_account(who: &TestClient) -> impl Future<Output = TestResponse> + Send + 'static {
    let who = who.clone();
    async move {
        who.delete_json("/api/auth/me", json!({"password": PASSWORD}))
            .await
    }
}

/// Removing a member who is deleting their account: the deletion locks every club of the user
/// first (conventions §5.6), like the removal, so the two run one after the other in either
/// order instead of deadlocking on the membership and removal rows.
#[sqlx::test]
async fn a_removal_and_the_members_account_deletion_take_turns(db: PgPool) {
    let app = TestApp::new(db).await;
    for (tag, removal_first) in [("a", true), ("b", false)] {
        let owner = app.user(&format!("owner_{tag}")).await;
        let leaver = app.user(&format!("leaver_{tag}")).await;
        let club = app.club(&owner, "Readers").await;
        app.join(&club, &leaver, "member").await;
        let removal = {
            let owner = owner.clone();
            let path = format!("/api/clubs/{}/members/{}", club.slug, leaver.username());
            async move { owner.delete(&path).await }
        };
        if removal_first {
            let (removed, deleted) = in_order(&app, &club, removal, delete_account(&leaver)).await;
            removed.assert_status(204);
            deleted.assert_status(204);
        } else {
            let (deleted, removed) = in_order(&app, &club, delete_account(&leaver), removal).await;
            deleted.assert_status(204);
            removed.assert_detail(404, "No member with that name");
        }
        assert_eq!(
            app.scalar_i64(&format!(
                "SELECT count(*) FROM users WHERE id = {}",
                leaver.id()
            ))
            .await,
            0
        );
        assert_eq!(owners(&app, &club).await, [owner.username()]);
    }
}

/// Deleting a club while one of its members deletes their account: both lock the club first.
#[sqlx::test]
async fn a_club_delete_and_a_members_account_deletion_take_turns(db: PgPool) {
    let app = TestApp::new(db).await;
    for (tag, club_first) in [("a", true), ("b", false)] {
        let owner = app.user(&format!("owner_{tag}")).await;
        let leaver = app.user(&format!("leaver_{tag}")).await;
        let club = app.club(&owner, "Readers").await;
        app.join(&club, &leaver, "admin").await;
        // The leaver's invite: the deletion clears its creator, the club delete removes it.
        leaver
            .post(
                &format!("/api/clubs/{}/invites", club.slug),
                json!({"kind": "link"}),
            )
            .await
            .assert_status(201);
        let club_delete = {
            let owner = owner.clone();
            let path = format!("/api/clubs/{}", club.slug);
            async move { owner.delete(&path).await }
        };
        let (first, second) = if club_first {
            in_order(&app, &club, club_delete, delete_account(&leaver)).await
        } else {
            in_order(&app, &club, delete_account(&leaver), club_delete).await
        };
        first.assert_status(204);
        second.assert_status(204);
        assert_eq!(
            app.scalar_i64(&format!(
                "SELECT (SELECT count(*) FROM clubs WHERE id = {0})
                      + (SELECT count(*) FROM users WHERE id = {1})",
                club.id,
                leaver.id()
            ))
            .await,
            0
        );
    }
}

/// The deferred owner triggers are the backstop: a transaction that would leave an existing
/// club without an owner fails at COMMIT, not at the statement.
#[sqlx::test]
async fn a_club_without_an_owner_fails_at_commit(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let member = app.user("member").await;
    let club = app.club(&owner, "Readers").await;
    app.join(&club, &member, "member").await;

    for statement in [
        "DELETE FROM club_memberships WHERE club_id = $1 AND role = 'owner'",
        "UPDATE club_memberships SET role = 'admin' WHERE club_id = $1 AND role = 'owner'",
    ] {
        let mut tx = app.db.begin().await.unwrap();
        let affected = sqlx::query(statement)
            .bind(club.id)
            .execute(&mut *tx)
            .await
            .expect("the statement itself succeeds")
            .rows_affected();
        assert_eq!(affected, 1);
        let error = tx.commit().await.expect_err("COMMIT must fail");
        let db_error = error.as_database_error().expect("a database error");
        assert_eq!(
            db_error.constraint(),
            Some("clubs_have_owner"),
            "{statement}"
        );
    }
    // A club created without its owner row fails the same way.
    let mut tx = app.db.begin().await.unwrap();
    sqlx::query("INSERT INTO clubs (slug, name) VALUES ('ownerless-abcd', 'Ownerless')")
        .execute(&mut *tx)
        .await
        .unwrap();
    let error = tx.commit().await.expect_err("COMMIT must fail");
    assert_eq!(
        error.as_database_error().unwrap().constraint(),
        Some("clubs_have_owner")
    );
    assert_eq!(owners(&app, &club).await, ["owner"]);
    // Deleting the whole club passes: the club is gone with its memberships.
    app.exec(&format!("DELETE FROM clubs WHERE id = {}", club.id))
        .await;
}

/// A pick of the club with a post by `author` in its discussion (POSTS arrives in Wave 2, so the
/// rows are inserted directly).
async fn club_post(app: &TestApp, club: &TestClub, author: &TestClient) {
    let work = app.work("Circe").await;
    app.exec(&format!(
        "WITH pick AS (
             INSERT INTO club_picks (club_id, work_id) VALUES ({c}, {w}) RETURNING id
         )
         INSERT INTO posts (work_id, author_id, club_id, pick_id, body)
         SELECT {w}, {a}, {c}, id, 'club talk' FROM pick",
        c = club.id,
        w = work.id,
        a = author.id()
    ))
    .await;
}

/// Deleting a club while its creator, who handed it over and left but still has a post in it,
/// deletes their account. The deletion clears `clubs.created_by` and deletes the post, so it
/// locks the club before the account row, like the club delete (§5.6): the two take turns
/// instead of each waiting for a row the other holds.
#[sqlx::test]
async fn a_club_delete_and_the_deletion_of_its_former_creator_take_turns(db: PgPool) {
    let app = TestApp::new(db).await;
    for (tag, club_first) in [("a", true), ("b", false)] {
        let creator = app.user(&format!("creator_{tag}")).await;
        let heir = app.user(&format!("heir_{tag}")).await;
        let club = app.club(&creator, "Readers").await;
        app.join(&club, &heir, "member").await;
        club_post(&app, &club, &creator).await;
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
        let club_delete = {
            let heir = heir.clone();
            let path = format!("/api/clubs/{}", club.slug);
            async move { heir.delete(&path).await }
        };
        let (first, second) = if club_first {
            in_order(&app, &club, club_delete, delete_account(&creator)).await
        } else {
            in_order(&app, &club, delete_account(&creator), club_delete).await
        };
        first.assert_status(204);
        second.assert_status(204);
        assert_eq!(
            app.scalar_i64(&format!(
                "SELECT (SELECT count(*) FROM clubs WHERE id = {0})
                      + (SELECT count(*) FROM users WHERE id = {1})",
                club.id,
                creator.id()
            ))
            .await,
            0
        );
    }
}

/// A role change the old owner sent while the transfer held the club runs on the roles under
/// the lock (§5.3): the extractor read "owner", but the caller is an admin by then.
#[sqlx::test]
async fn a_role_change_by_the_old_owner_after_the_transfer_is_refused(db: PgPool) {
    let app = TestApp::new(db).await;
    let s = Succession::new(&app, "a").await;
    let demote = {
        let owner = s.owner.clone();
        let path = format!("/api/clubs/{}/members/{}", s.club.slug, s.admin.username());
        async move { owner.patch(&path, json!({"role": "member"})).await }
    };
    let (transfer, changed) = in_order(&app, &s.club, s.transfer(), demote).await;
    transfer.assert_status(200);
    changed.assert_detail(403, "Only the club owner can do that");
    assert_eq!(
        role_of(&app, &s.club, &s.admin).await.as_deref(),
        Some("admin")
    );
    assert_eq!(s.owners(&app).await, ["heir_a"]);
}

/// A club delete the old owner sent while the transfer held the club is refused: the delete
/// checks the owner role in its statement, under the lock.
#[sqlx::test]
async fn a_club_delete_by_the_old_owner_after_the_transfer_is_refused(db: PgPool) {
    let app = TestApp::new(db).await;
    let s = Succession::new(&app, "a").await;
    let delete = {
        let owner = s.owner.clone();
        let path = format!("/api/clubs/{}", s.club.slug);
        async move { owner.delete(&path).await }
    };
    let (transfer, deleted) = in_order(&app, &s.club, s.transfer(), delete).await;
    transfer.assert_status(200);
    deleted.assert_detail(403, "Only the club owner can do that");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM clubs WHERE id = {}",
            s.club.id
        ))
        .await,
        1
    );
    assert_eq!(s.owners(&app).await, ["heir_a"]);
}

/// An admin removed while their own removal of someone else waited for the club is no longer a
/// member under the lock: 404 `No such club`, and nobody else is removed.
#[sqlx::test]
async fn a_write_by_an_admin_removed_meanwhile_is_no_such_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let s = Succession::new(&app, "a").await;
    let (removed, refused) =
        in_order(&app, &s.club, s.remove_admin(), s.remove_heir(&s.admin)).await;
    removed.assert_status(204);
    refused.assert_detail(404, "No such club");
    assert_eq!(
        role_of(&app, &s.club, &s.heir).await.as_deref(),
        Some("member")
    );
    assert_eq!(role_of(&app, &s.club, &s.admin).await, None);
}

impl Succession {
    fn remove_admin(&self) -> impl Future<Output = TestResponse> + Send + 'static {
        let owner = self.owner.clone();
        let path = format!(
            "/api/clubs/{}/members/{}",
            self.club.slug,
            self.admin.username()
        );
        async move { owner.delete(&path).await }
    }

    fn create_link(&self) -> impl Future<Output = TestResponse> + Send + 'static {
        let admin = self.admin.clone();
        let path = format!("/api/clubs/{}/invites", self.club.slug);
        async move { admin.post(&path, json!({"kind": "link"})).await }
    }
}

async fn invites_by(app: &TestApp, who: &TestClient) -> i64 {
    app.scalar_i64(&format!(
        "SELECT count(*) FROM club_invites WHERE created_by = {}",
        who.id()
    ))
    .await
}

/// Invite writes lock the club and re-read the caller's role (§5.3): an admin removed while
/// their request waited gets no invite to walk back in with, and revokes nothing; a demoted
/// admin gets 403.
#[sqlx::test]
async fn an_admin_removed_or_demoted_meanwhile_cannot_create_or_revoke_invites(db: PgPool) {
    let app = TestApp::new(db).await;

    let s = Succession::new(&app, "a").await;
    let (removed, created) = in_order(&app, &s.club, s.remove_admin(), s.create_link()).await;
    removed.assert_status(204);
    created.assert_detail(404, "No such club");
    assert_eq!(invites_by(&app, &s.admin).await, 0);

    let s = Succession::new(&app, "b").await;
    let invite = s
        .owner
        .post(
            &format!("/api/clubs/{}/invites", s.club.slug),
            json!({"kind": "link"}),
        )
        .await
        .json();
    // Expired: the removal revokes only active invites, so a revocation would be the removed
    // admin's own.
    app.exec(&format!(
        "UPDATE club_invites SET expires_at = now() - interval '1 minute' WHERE id = {}",
        invite["id"]
    ))
    .await;
    let revoke = {
        let admin = s.admin.clone();
        let path = format!("/api/clubs/{}/invites/{}", s.club.slug, invite["id"]);
        async move { admin.delete(&path).await }
    };
    let (removed, revoked) = in_order(&app, &s.club, s.remove_admin(), revoke).await;
    removed.assert_status(204);
    revoked.assert_detail(404, "No such club");
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_invites WHERE revoked_at IS NOT NULL")
            .await,
        0
    );

    let s = Succession::new(&app, "c").await;
    let demote = {
        let owner = s.owner.clone();
        let path = format!("/api/clubs/{}/members/{}", s.club.slug, s.admin.username());
        async move { owner.patch(&path, json!({"role": "member"})).await }
    };
    let (demoted, created) = in_order(&app, &s.club, demote, s.create_link()).await;
    demoted.assert_status(200);
    created.assert_detail(403, "Only club admins can do that");
    assert_eq!(invites_by(&app, &s.admin).await, 0);
}

/// A removal and a new invite are dated while they hold the club lock, not when their
/// transaction began, so the order of the two dates is the order of the two writes: an invite
/// created before the removal is refused to the removed member, one created after it works.
#[sqlx::test]
async fn removals_and_invites_are_dated_under_the_club_lock(db: PgPool) {
    let app = TestApp::new(db).await;

    // The invite first: its creator is removed afterwards and cannot use it.
    let s = Succession::new(&app, "a").await;
    let (created, removed) = in_order(&app, &s.club, s.create_link(), s.remove_admin()).await;
    created.assert_status(201);
    removed.assert_status(204);
    s.admin
        .post(
            &format!(
                "/api/invites/{}/accept",
                created.json()["code"].as_str().unwrap()
            ),
            json!({}),
        )
        .await
        .assert_detail(400, "That invite code is not valid");

    // The removal first, and both requests began before the lock was released: the removal is
    // dated after the release, and the invite (another admin's) after the removal.
    let s = Succession::new(&app, "b").await;
    let mut hold = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM clubs WHERE id = $1 FOR UPDATE")
        .bind(s.club.id)
        .execute(&mut *hold)
        .await
        .unwrap();
    let removal = tokio::spawn(s.remove_heir(&s.owner));
    wait_for_waiters(&app, 1).await;
    let invite = tokio::spawn(s.create_link());
    wait_for_waiters(&app, 2).await;
    let released: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *hold)
        .await
        .unwrap();
    hold.commit().await.unwrap();
    removal.await.unwrap().assert_status(204);
    let invite = invite.await.unwrap();
    invite.assert_status(201);
    let (removed_at, created_at): (OffsetDateTime, OffsetDateTime) = sqlx::query_as(
        "SELECT r.removed_at, i.created_at FROM club_removals r, club_invites i
         WHERE r.club_id = $1 AND r.user_id = $2 AND i.id = $3",
    )
    .bind(s.club.id)
    .bind(s.heir.id())
    .bind(invite.json()["id"].as_i64().unwrap())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(released < removed_at, "{released} < {removed_at}");
    assert!(removed_at < created_at, "{removed_at} < {created_at}");
    s.heir
        .post(
            &format!(
                "/api/invites/{}/accept",
                invite.json()["code"].as_str().unwrap()
            ),
            json!({}),
        )
        .await
        .assert_status(200);
}
