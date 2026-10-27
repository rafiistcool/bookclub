//! Races on posts (conventions §13, docs/phase1/spec/diary.md R3 and R12): a delete and a reply
//! or reaction in flight are ordered by the post's row lock, so a reply is never lost under a
//! deleted parent and a delete never fails; reaction toggles are atomic. Club writes take the
//! club row first, then the work (against a work merge), then the post.
//!
//! The deterministic tests hold the lock in a transaction of their own, start the request, wait
//! until it waits for that lock, and then commit.

mod common;

use std::time::Duration;

use bookclub_server::catalog::merge_works;
use common::*;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};

/// Removes `grace`, the only plain member of `club_setup`'s club.
const REMOVAL: &str = "DELETE FROM club_memberships WHERE club_id = $1 AND role = 'member'";

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

/// Waits until `n` sessions wait for a lock or `task` has finished (a request that takes no
/// conflicting lock runs straight through), for at most five seconds.
async fn wait_for_waiters_or_end<T>(app: &TestApp, n: i64, task: &tokio::task::JoinHandle<T>) {
    for _ in 0..500 {
        let waiting = app
            .scalar_i64(
                "SELECT count(*) FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event_type = 'Lock'",
            )
            .await;
        if waiting >= n || task.is_finished() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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

/// One reader double-clicks on a club post: both toggles answer 200 (never a 500 from the
/// primary key), and each round records exactly one `reaction` event. Two concurrent toggles
/// store exactly one insert in every interleaving: from absent, one inserts and the other
/// finds nothing to delete and conflicts (or deletes the first one's row); from present, one
/// deletes and the other inserts. An event recorded for an insert that did nothing would make
/// a round count two.
#[sqlx::test]
async fn a_double_toggle_never_fails(db: PgPool) {
    let app = TestApp::new(db).await;
    let (grace, _, _, post) = club_setup(&app).await;
    let url = format!("/api/posts/{post}/reactions");
    for round in 1..=5 {
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
        assert_eq!(
            app.scalar_i64("SELECT count(*) FROM events WHERE kind = 'reaction'")
                .await,
            round,
            "round {round}"
        );
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
    under_the_club_lock(&app, &club, removed, REMOVAL)
        .await
        .assert_detail(404, "No such club");
    app.join(&club, &grace, "member").await;
    let deleted = async move { grace.post(&path, json!({"body": "Late"})).await };
    under_the_club_lock(&app, &club, deleted, "DELETE FROM clubs WHERE id = $1")
        .await
        .assert_detail(404, "No such club");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM posts").await, 0);
}

/// A member removed while their reaction waited for the club row gets 404 (as for any post
/// they cannot see) and nothing is stored: the access read again under that lock refuses it
/// before the toggle, and the answer's own read (`post_out`, in the same transaction) would
/// refuse it after the toggle and roll it back.
#[sqlx::test]
async fn a_reaction_on_a_club_post_waits_for_a_removal(db: PgPool) {
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
    under_the_club_lock(&app, &club, reaction, REMOVAL)
        .await
        .assert_detail(404, "No entry with that id");
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM post_reactions").await,
        0
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM events WHERE kind = 'reaction'")
            .await,
        0
    );
}

/// A member removed while their diary-route reply to a club post waited for the club row: the
/// parent, read again under that lock, is no longer theirs to reply to.
#[sqlx::test]
async fn a_reply_to_a_club_post_waits_for_a_removal(db: PgPool) {
    let app = TestApp::new(db).await;
    let (grace, club, circe, post) = club_setup(&app).await;
    let reply = async move {
        grace
            .post(&thread(&circe), json!({"body": "Reply", "parent_id": post}))
            .await
    };
    under_the_club_lock(&app, &club, reply, REMOVAL)
        .await
        .assert_detail(400, "That entry is not on this book");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM posts").await, 1);
    // Only ada's post recorded `note_posted`.
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM events WHERE kind = 'note_posted'")
            .await,
        1
    );
}

/// A milestone deleted while a post naming it is written: the post waits for the delete and is
/// refused like any milestone that is not on the pick, never a foreign-key 500.
#[sqlx::test]
async fn a_post_on_a_milestone_waits_for_its_delete(db: PgPool) {
    let app = TestApp::new(db).await;
    let (grace, club, _, _) = club_setup(&app).await;
    let milestone: i64 = sqlx::query_scalar(
        "INSERT INTO milestones (club_id, pick_id, title)
         SELECT club_id, id, 'Part one' FROM club_picks WHERE club_id = $1
         RETURNING id",
    )
    .bind(club.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    let mut hold = app.db.begin().await.unwrap();
    sqlx::query("DELETE FROM milestones WHERE id = $1")
        .bind(milestone)
        .execute(&mut *hold)
        .await
        .unwrap();
    let posting = {
        let path = format!("/api/clubs/{}/pick/posts", club.slug);
        tokio::spawn(async move {
            grace
                .post(
                    &path,
                    json!({"body": "On part one", "milestone_id": milestone}),
                )
                .await
        })
    };
    wait_for_waiters(&app, 1).await;
    hold.commit().await.unwrap();
    posting
        .await
        .unwrap()
        .assert_detail(400, "That milestone is not on this pick");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM posts").await, 1);
}

/// Runs CAT's `merge_works` of `loser` into `winner` in a transaction left open, so the merge
/// holds its locks (both works `FOR NO KEY UPDATE`, the loser's readers `FOR UPDATE`, every row
/// it moved) until the caller commits.
async fn merge_held(app: &TestApp, loser: i64, winner: i64) -> Transaction<'static, Postgres> {
    let mut hold = app.db.begin().await.unwrap();
    merge_works(&mut hold, loser, winner).await.unwrap();
    hold
}

/// The first steps of `merge_works`, for a request to start between them and the moves: both
/// works `FOR NO KEY UPDATE`, then the users it moves shelf rows for `FOR UPDATE`. The real
/// function cannot be paused there.
async fn begin_merge(
    app: &TestApp,
    loser: i64,
    winner: i64,
    users: &[i64],
) -> Transaction<'static, Postgres> {
    let mut hold = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM works WHERE id = ANY($1) ORDER BY id FOR NO KEY UPDATE")
        .bind(vec![loser, winner])
        .execute(&mut *hold)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM users WHERE id = ANY($1) ORDER BY id FOR UPDATE")
        .bind(users.to_vec())
        .execute(&mut *hold)
        .await
        .unwrap();
    hold
}

/// The statements of `merge_works`' `move_references` that re-point the loser's posts, picks
/// and events (one statement per table, so a thread moves as a whole), then the redirect.
async fn move_work(hold: &mut PgConnection, loser: i64, winner: i64) -> sqlx::Result<()> {
    for sql in [
        "UPDATE posts SET work_id = $2 WHERE work_id = $1",
        "UPDATE events SET work_id = $2 WHERE work_id = $1",
        "UPDATE club_picks SET work_id = $2 WHERE work_id = $1",
        "UPDATE works SET merged_into_id = $2, updated_at = now() WHERE id = $1",
    ] {
        sqlx::query(sql)
            .bind(loser)
            .bind(winner)
            .execute(&mut *hold)
            .await?;
    }
    Ok(())
}

/// The work a post is on.
async fn work_of(app: &TestApp, post: &Value) -> i64 {
    app.scalar_i64(&format!(
        "SELECT work_id FROM posts WHERE id = {}",
        post["id"]
    ))
    .await
}

/// An entry written while its work is merged away waits for the merge and lands on the winner;
/// without the work lock it would stay on the loser, in no thread.
#[sqlx::test]
async fn an_entry_written_during_a_merge_lands_on_the_winner(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = app.work("Circe").await;
    let winner = app.work("Circe (catalog)").await;
    let hold = merge_held(&app, circe.id, winner.id).await;
    let writing = {
        let (ada, path) = (ada.clone(), thread(&circe));
        tokio::spawn(async move { ada.post(&path, json!({"body": "Mid-merge"})).await })
    };
    wait_for_waiters_or_end(&app, 1, &writing).await;
    hold.commit().await.unwrap();
    let res = writing.await.unwrap();
    res.assert_status(201);
    assert_eq!(work_of(&app, &res.json()).await, winner.id);
    let diary = ada.get(&thread(&circe)).await.json();
    assert_eq!(diary["work_id"], winner.id);
    assert_eq!(diary["items"][0]["body"], "Mid-merge");
}

/// A reply written while its thread's work is merged away follows its parent to the winner
/// (without the work lock it found the parent moved and answered 400).
#[sqlx::test]
async fn a_reply_written_during_a_merge_follows_its_parent(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace, circe, root) = setup(&app).await;
    let winner = app.work("Circe (catalog)").await;
    let hold = merge_held(&app, circe.id, winner.id).await;
    let replying = {
        let path = thread(&circe);
        tokio::spawn(async move {
            grace
                .post(&path, json!({"body": "Mid-merge", "parent_id": root}))
                .await
        })
    };
    wait_for_waiters_or_end(&app, 1, &replying).await;
    hold.commit().await.unwrap();
    let res = replying.await.unwrap();
    res.assert_status(201);
    assert_eq!(work_of(&app, &res.json()).await, winner.id);
    let diary = ada.get(&thread(&winner)).await.json();
    assert_eq!(diary["items"][0]["replies"][0]["body"], "Mid-merge");
}

/// A pick post written while the pick's work is merged away lands on the work the pick moved
/// to, so its thread keeps taking replies (without the work lock it stayed on the loser and a
/// reply to it was 400 `That entry is not on this book`).
#[sqlx::test]
async fn a_pick_post_written_during_a_merge_follows_its_pick(db: PgPool) {
    let app = TestApp::new(db).await;
    let (grace, club, circe, _) = club_setup(&app).await;
    let winner = app.work("Circe (catalog)").await;
    let path = format!("/api/clubs/{}/pick/posts", club.slug);
    let hold = merge_held(&app, circe.id, winner.id).await;
    let posting = {
        let (grace, path) = (grace.clone(), path.clone());
        tokio::spawn(async move { grace.post(&path, json!({"body": "Mid-merge"})).await })
    };
    wait_for_waiters_or_end(&app, 1, &posting).await;
    hold.commit().await.unwrap();
    let res = posting.await.unwrap();
    res.assert_status(201);
    let post = res.json();
    assert_eq!(work_of(&app, &post).await, winner.id);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT work_id FROM club_picks WHERE club_id = {}",
            club.id
        ))
        .await,
        winner.id
    );
    grace
        .post(&path, json!({"body": "Reply", "parent_id": post["id"]}))
        .await
        .assert_status(201);
    let items = grace.get(&path).await.json()["items"].clone();
    assert_eq!(items[1]["body"], "Mid-merge");
    assert_eq!(items[1]["replies"][0]["body"], "Reply");
}

/// A reaction that meets a merge of the post's work waits for it (the work lock comes before
/// the post) and records its event on the winner. Locking the post first deadlocked: the merge
/// holds the reader's user row and needs the post to move it, the reaction holds the post and
/// needs the user row for its foreign key. The merge runs in steps (`begin_merge`, then
/// `move_work`) so that the reaction starts between its user locks and its `UPDATE posts`.
#[sqlx::test]
async fn a_reaction_during_a_merge_waits_instead_of_deadlocking(db: PgPool) {
    let app = TestApp::new(db).await;
    let (grace, _, circe, post) = club_setup(&app).await;
    let winner = app.work("Circe (catalog)").await;
    let mut hold = begin_merge(&app, circe.id, winner.id, &[grace.id()]).await;
    let reacting = tokio::spawn(async move {
        grace
            .post(
                &format!("/api/posts/{post}/reactions"),
                json!({"emoji": HEART}),
            )
            .await
    });
    wait_for_waiters(&app, 1).await;
    let moved = move_work(&mut hold, circe.id, winner.id).await;
    assert!(moved.is_ok(), "the merge failed: {moved:?}");
    hold.commit().await.unwrap();
    reacting.await.unwrap().assert_status(200);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM post_reactions").await,
        1
    );
    assert_eq!(
        app.scalar_i64("SELECT work_id FROM events WHERE kind = 'reaction'")
            .await,
        winner.id
    );
}
