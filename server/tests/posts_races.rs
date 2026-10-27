//! Races on posts (conventions §13, docs/phase1/spec/diary.md R3 and R12): a delete and a reply
//! or reaction in flight are ordered by the post's row lock, so a reply is never lost under a
//! deleted parent and a delete never fails; reaction toggles are atomic.
//!
//! The deterministic tests hold the lock in a transaction of their own, start the request, wait
//! until it waits for that lock, and then commit.

mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const HEART: &str = "\u{2764}\u{fe0f}";

fn thread(work: &TestWork) -> String {
    format!("/api/books/works/{}/posts", work.id)
}

/// `ada` and `grace` in one club, and ada's entry on Circe.
async fn setup(app: &TestApp) -> (TestClient, TestClient, TestWork, i64) {
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let circe = app.work("Circe").await;
    let res = ada.post(&thread(&circe), json!({"body": "Root"})).await;
    res.assert_status(201);
    let root = res.json()["id"].as_i64().unwrap();
    (ada, grace, circe, root)
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
async fn a_delete_waits_for_a_reply_in_flight_and_leaves_a_tombstone(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, circe, root) = setup(&app).await;
    // A reply in flight holds its parent FOR KEY SHARE until it commits.
    let mut hold = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM posts WHERE id = $1 FOR KEY SHARE")
        .bind(root)
        .execute(&mut *hold)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO posts (work_id, author_id, parent_id, body) VALUES ($1, $2, $3, 'In flight')",
    )
    .bind(circe.id)
    .bind(grace.id())
    .bind(root)
    .execute(&mut *hold)
    .await
    .unwrap();
    let deleting = {
        let ada = ada.clone();
        tokio::spawn(async move { ada.delete(&format!("/api/posts/{root}")).await })
    };
    wait_for_waiters(&app, 1).await;
    hold.commit().await.unwrap();
    deleting.await.unwrap().assert_status(204);
    let diary = ada.get(&thread(&circe)).await.json();
    assert_eq!(diary["items"][0]["deleted"], true);
    assert_eq!(diary["items"][0]["replies"][0]["body"], "In flight");
}

#[sqlx::test]
async fn a_reply_waits_for_a_delete_in_flight(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, grace, circe, root) = setup(&app).await;
    for (finish, detail) in [
        (
            "UPDATE posts SET body = '', spoiler_upto = NULL, deleted_at = now() WHERE id = $1",
            "That entry was deleted",
        ),
        (
            "DELETE FROM posts WHERE id = $1",
            "That entry is not on this book",
        ),
    ] {
        let mut hold = app.db.begin().await.unwrap();
        sqlx::query("SELECT id FROM posts WHERE id = $1 FOR UPDATE")
            .bind(root)
            .execute(&mut *hold)
            .await
            .unwrap();
        let replying = {
            let grace = grace.clone();
            let path = thread(&circe);
            tokio::spawn(async move {
                grace
                    .post(&path, json!({"body": "Too late", "parent_id": root}))
                    .await
            })
        };
        wait_for_waiters(&app, 1).await;
        sqlx::query(finish)
            .bind(root)
            .execute(&mut *hold)
            .await
            .unwrap();
        hold.commit().await.unwrap();
        replying.await.unwrap().assert_detail(400, detail);
    }
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM posts WHERE parent_id IS NOT NULL")
            .await,
        0
    );
}

#[sqlx::test]
async fn a_reaction_waits_for_a_delete_in_flight(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, grace, _, root) = setup(&app).await;
    let mut hold = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM posts WHERE id = $1 FOR UPDATE")
        .bind(root)
        .execute(&mut *hold)
        .await
        .unwrap();
    let reacting = {
        let grace = grace.clone();
        tokio::spawn(async move {
            grace
                .post(
                    &format!("/api/posts/{root}/reactions"),
                    json!({"emoji": HEART}),
                )
                .await
        })
    };
    wait_for_waiters(&app, 1).await;
    sqlx::query("DELETE FROM posts WHERE id = $1")
        .bind(root)
        .execute(&mut *hold)
        .await
        .unwrap();
    hold.commit().await.unwrap();
    reacting
        .await
        .unwrap()
        .assert_detail(404, "No entry with that id");
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM post_reactions").await,
        0
    );
}

/// Many readers react at once: every toggle lands, and each records one event.
#[sqlx::test]
async fn concurrent_reactions_all_land(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let circe = app.work("Circe").await;
    sqlx::query("INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2)")
        .bind(club.id)
        .bind(circe.id)
        .execute(&app.db)
        .await
        .unwrap();
    let mut readers = Vec::new();
    for n in 0..8 {
        let reader = app.user(&format!("reader{n}")).await;
        app.join(&club, &reader, "member").await;
        readers.push(reader);
    }
    let post = ada
        .post(
            &format!("/api/clubs/{}/pick/posts", club.slug),
            json!({"body": "React!"}),
        )
        .await
        .json();
    let url = format!("/api/posts/{}/reactions", post["id"]);
    let tasks: Vec<_> = readers
        .iter()
        .cloned()
        .map(|reader| {
            let url = url.clone();
            tokio::spawn(async move { reader.post(&url, json!({"emoji": HEART})).await })
        })
        .collect();
    for task in tasks {
        task.await.unwrap().assert_status(200);
    }
    let after = ada
        .get(&format!("/api/clubs/{}/pick/posts", club.slug))
        .await;
    assert_eq!(after.json()["items"][0]["reactions"][0]["count"], 8);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM events WHERE kind = 'reaction'")
            .await,
        8
    );
}

/// One reader double-clicks: both toggles answer 200 and at most one reaction (and at most
/// one event per stored reaction) remains; never a 500 from the unique key.
#[sqlx::test]
async fn a_double_toggle_never_fails(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, grace, _, root) = setup(&app).await;
    let url = format!("/api/posts/{root}/reactions");
    for _ in 0..5 {
        let tasks: Vec<_> = (0..2)
            .map(|_| {
                let grace = grace.clone();
                let url = url.clone();
                tokio::spawn(async move { grace.post(&url, json!({"emoji": HEART})).await })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().assert_status(200);
        }
        let count = app.scalar_i64("SELECT count(*) FROM post_reactions").await;
        assert!(count <= 1, "{count} reactions");
    }
}

/// Parallel replies and a delete of their parent: every reply that answered 201 hangs under a
/// tombstone, every other one was refused with a 400, and nothing failed.
#[sqlx::test]
async fn replies_racing_a_delete_are_kept_or_refused(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, circe, root) = setup(&app).await;
    let mut replies = Vec::new();
    for n in 0..6 {
        let grace = grace.clone();
        let path = thread(&circe);
        replies.push(tokio::spawn(async move {
            grace
                .post(
                    &path,
                    json!({"body": format!("Reply {n}"), "parent_id": root}),
                )
                .await
        }));
    }
    let deleted = ada.delete(&format!("/api/posts/{root}")).await;
    deleted.assert_status(204);
    let mut kept = 0;
    for reply in replies {
        let res = reply.await.unwrap();
        match res.status.as_u16() {
            201 => kept += 1,
            400 => assert!(
                ["That entry was deleted", "That entry is not on this book"]
                    .contains(&res.detail().as_str()),
                "{}",
                res.text()
            ),
            other => panic!("{other}: {}", res.text()),
        }
    }
    let diary = ada.get(&thread(&circe)).await.json();
    let items = diary["items"].as_array().unwrap();
    if kept == 0 {
        assert_eq!(*items, Vec::<Value>::new());
    } else {
        assert_eq!(items[0]["deleted"], true);
        assert_eq!(items[0]["replies"].as_array().unwrap().len(), kept);
    }
}

/// `ada`'s club `Readers` with `grace` as a member, Circe as the current pick and a post of
/// ada's on it.
async fn club_setup(app: &TestApp) -> (TestClient, TestClub, TestWork, i64) {
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let circe = app.work("Circe").await;
    sqlx::query("INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2)")
        .bind(club.id)
        .bind(circe.id)
        .execute(&app.db)
        .await
        .unwrap();
    let res = ada
        .post(
            &format!("/api/clubs/{}/pick/posts", club.slug),
            json!({"body": "Club post"}),
        )
        .await;
    res.assert_status(201);
    (grace, club, circe, res.json()["id"].as_i64().unwrap())
}

/// Holds the club row `FOR UPDATE` (as a club delete, a removal and an account deletion do),
/// starts `request`, waits until it waits for that lock, runs `then` in the holding
/// transaction and commits. The request must not have locked a post of the club first: the
/// held transaction's `then` would deadlock with it.
async fn under_the_club_lock(
    app: &TestApp,
    club: &TestClub,
    request: impl std::future::Future<Output = TestResponse> + Send + 'static,
    then: &'static str,
) -> TestResponse {
    let mut hold = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM clubs WHERE id = $1 FOR UPDATE")
        .bind(club.id)
        .execute(&mut *hold)
        .await
        .unwrap();
    let task = tokio::spawn(request);
    wait_for_waiters(app, 1).await;
    sqlx::query(then)
        .bind(club.id)
        .execute(&mut *hold)
        .await
        .unwrap();
    hold.commit().await.unwrap();
    task.await.unwrap()
}

#[sqlx::test]
async fn a_reaction_on_a_club_post_waits_for_a_club_delete(db: PgPool) {
    let app = TestApp::new(db).await;
    let (grace, club, _, post) = club_setup(&app).await;
    let reaction = async move {
        grace
            .post(
                &format!("/api/posts/{post}/reactions"),
                json!({"emoji": HEART}),
            )
            .await
    };
    under_the_club_lock(&app, &club, reaction, "DELETE FROM clubs WHERE id = $1")
        .await
        .assert_detail(404, "No entry with that id");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM posts").await, 0);
}

#[sqlx::test]
async fn a_reply_to_a_club_post_waits_for_a_club_delete(db: PgPool) {
    let app = TestApp::new(db).await;
    let (grace, club, circe, post) = club_setup(&app).await;
    let reply = async move {
        grace
            .post(&thread(&circe), json!({"body": "Reply", "parent_id": post}))
            .await
    };
    under_the_club_lock(&app, &club, reply, "DELETE FROM clubs WHERE id = $1")
        .await
        .assert_detail(400, "That entry is not on this book");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM posts").await, 0);
}

#[sqlx::test]
async fn a_club_post_waits_for_a_club_delete_or_a_removal(db: PgPool) {
    let app = TestApp::new(db).await;
    let (grace, club, _, post) = club_setup(&app).await;
    let path = format!("/api/clubs/{}/pick/posts", club.slug);
    let removed = {
        let (grace, path) = (grace.clone(), path.clone());
        async move {
            grace
                .post(&path, json!({"body": "Reply", "parent_id": post}))
                .await
        }
    };
    // `grace` is the club's only plain member.
    let removal = "DELETE FROM club_memberships WHERE club_id = $1 AND role = 'member'";
    under_the_club_lock(&app, &club, removed, removal)
        .await
        .assert_detail(404, "No such club");
    app.join(&club, &grace, "member").await;
    let deleted = async move { grace.post(&path, json!({"body": "Late"})).await };
    under_the_club_lock(&app, &club, deleted, "DELETE FROM clubs WHERE id = $1")
        .await
        .assert_detail(404, "No such club");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM posts").await, 0);
}
